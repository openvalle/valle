use super::*;
use crate::{frame::RenderQuality, prepare::motion_kernel::MotionKernel, render::*, resource::*};
use serde_json::json;
use valle_timeline::internal::{
    decode_canonical, decode_resource_manifest, wire::resource::ResourceEntryWire,
};

fn render() -> EngineRender {
    let artifact = Arc::new(
        valle_compiler::motion::compile_motion(
            r##"
export const composition = { width: 320, height: 180, duration: 1 };
export default function Card() { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return <Scene style={{ width: 320, height: 180, backgroundColor: "#123456" }}>
    <View style={{ width: 30, height: 40, left: frame, backgroundColor: "#ff0033" }} />
  </Scene>;
}
"##,
        )
        .unwrap()
        .artifact,
    );
    let digest = ContentDigest::of_bytes(&valle_motion::canonical_bytes(&artifact).unwrap());
    let manifest = decode_resource_manifest(
        &serde_json::to_vec(&json!({"entries": {"component:card": {
            "kind":"motion-artifact", "digest":digest.to_string(), "abi":"valle.motion/artifact@1",
            "descriptor":{"readsDestination":false,"boundarySampling":"left-limit"}
        }}}))
        .unwrap(),
    )
    .unwrap();
    let ResourceEntryWire::MotionArtifact {
        abi,
        descriptor,
        digest,
    } = &manifest.entries()["component:card"]
    else {
        unreachable!()
    };
    let bindings = ResourceBindings::new()
        .with_binding(
            "component:card",
            ResourceBinding::new(
                *digest,
                VerifiedHandleId::new(1).unwrap(),
                VerifiedResourceFacts::MotionArtifact {
                    abi: *abi,
                    descriptor: descriptor.clone(),
                    artifact,
                    temporal_footprint: VisualFootprint::new(0, 0),
                },
            ),
        )
        .unwrap();
    let constant = |value| json!({"type":"constant","value":value});
    let clip = |id| {
        json!({"type":"clip","id":id,"duration":"1/1",
        "layer":{"transform":{"position":constant(json!([0.5,0.5])),"scale":constant(json!([1,1])),
            "rotation":constant(json!(0)),"anchor":[0.5,0.5]},"opacity":constant(json!(1)),"mask":null,"filters":[],"blend":"normal"},
        "source":{"type":"motion","component":"component:card","fit":"contain","sourceStart":"0/1",
            "role":{"type":"clip"},"sourceDuration":"1/1","rate":"1/1","endBehavior":"hold","props":{},"resources":{}}})
    };
    let timeline = decode_canonical(&json!({"document":{
        "canvas":{"width":320,"height":180,"fps":"30/1","sampleRate":48000,"channelLayout":"stereo","colorSpace":"srgb","duration":"2/1"},
        "background":{"color":"#000000ff"},"visual":{"tracks":[{"id":"track:main","items":[clip("clip:a"),
            {"type":"transition","id":"transition:mix","duration":"1/2","kernel":{"type":"example.transition/crossfade@1","parameters":{}}},clip("clip:b")]}]},
        "audio":{"tracks":[]},"adjustments":[],"captions":{"tracks":[]},"camera":null,"metadata":{}
    }}).to_string()).unwrap();
    let capabilities = Capabilities::new()
        .with_artifact_abi("valle.motion/artifact@1")
        .with_extension_kernel(
            "example.transition/crossfade@1",
            ExtensionKernelCapability::new(
                EXTENSION_CROSS_FADE_ABI,
                engine_owned_kernel_implementation_digest(EXTENSION_CROSS_FADE_ABI).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let profile = ExecutionProfile::from_projection(
        serde_json::from_str(
            &crate::fixed_package::canonical_fixed_execution_profile(
                crate::fixed_package::COMMON_PROFILE_KEY,
            )
            .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    EngineRender::open(&timeline, &manifest, &bindings, &capabilities, &profile).unwrap()
}

fn packet(
    jobs: Vec<crate::prepare::motion_kernel::MotionKernelJob>,
    compiler: &FrameCompiler,
    kernel: &mut MotionKernel,
) -> Vec<u8> {
    let mut output = Vec::new();
    for job in jobs {
        let bytes = kernel
            .run(
                &job.definition_id,
                compiler.motion_definition(&job.definition_id).unwrap(),
                &job.input_json,
                false,
            )
            .unwrap();
        for field in [job.key.as_bytes(), bytes.as_slice()] {
            output.extend_from_slice(&(field.len() as u32).to_le_bytes());
            output.extend_from_slice(field);
        }
    }
    output
}

#[test]
fn captured_transition_endpoints_resume_exactly_and_failure_does_not_poison_next_frame() {
    let render = render();
    let id = render.render_id();
    let mut direct = render.frame_compiler();
    let mut staged = render.frame_compiler();
    let mut kernel = MotionKernel::default();
    let spec = RenderSpec::new(
        320,
        180,
        RenderQuality::Preview,
        OutputSpec::srgb_preview(OutputBackground::opaque_srgb([0, 0, 0])).unwrap(),
    )
    .unwrap();
    let mut transition_seen = false;
    for frame in [29, 2, 35, 29, 59] {
        let key = FrameKey::new(frame);
        let jobs = staged.capture_motion_jobs(id, key, spec).unwrap();
        transition_seen |= jobs.len() == 2;
        let bytes = packet(jobs, &staged, &mut kernel);
        let expected = direct.evaluate_prepare(id, key, spec).unwrap();
        let actual = staged
            .evaluate_prepare_motion(id, key, spec, &bytes)
            .unwrap();
        assert_eq!(actual.prepared(), expected.prepared());
    }
    assert!(transition_seen);
    assert!(
        staged
            .evaluate_prepare_motion(id, FrameKey::new(29), spec, &[])
            .is_err()
    );
    assert!(staged.caches.motion_preparation.stage.is_none());
    assert_eq!(
        staged
            .evaluate_prepare(id, FrameKey::new(29), spec)
            .unwrap()
            .prepared(),
        direct
            .evaluate_prepare(id, FrameKey::new(29), spec)
            .unwrap()
            .prepared()
    );
    assert!(
        staged
            .capture_motion_jobs(id, FrameKey::new(1000), spec)
            .is_err()
    );
    assert!(staged.caches.motion_preparation.stage.is_none());
}
