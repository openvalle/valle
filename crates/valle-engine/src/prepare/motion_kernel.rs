//! Stateless Motion layout/emission jobs. The authoritative compiler alone owns the Timeline,
//! resource bindings and frame assembly; kernels receive one admitted scene and explicit inputs.
use super::motion::{
    self, BuiltMotionProgram, CompiledMotionProgramContext, MotionFrameInput, ProgramPrepareError,
};
use crate::resource::ContentDigest;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use valle_draw::{
    Rect,
    program::{DrawProgram, DrawProgramBuilder},
};

type Fonts = Vec<(ContentDigest, Arc<[u8]>, bool)>;
const MAX_DEFINITION_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MIN_KERNEL_NODES: usize = 128;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Definition {
    artifact: valle_motion::SceneArtifact,
    fonts: Vec<FontDefinition>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FontDefinition {
    digest: ContentDigest,
    bytes: String,
    generic: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MotionKernelJob {
    pub key: String,
    pub definition_id: String,
    pub input_json: String,
}

pub(crate) enum Stage {
    Capture(Vec<MotionKernelJob>),
    Complete(BTreeMap<String, BuiltMotionProgram>),
}

#[derive(Default)]
pub(crate) struct MotionPreparation {
    pub(crate) stage: Option<Stage>,
    definitions: VecDeque<(u32, String, String)>,
    oversized_sources: BTreeSet<u32>,
}

impl MotionPreparation {
    pub fn definition(&self, id: &str) -> Option<&str> {
        self.definitions
            .iter()
            .find(|(_, key, _)| key == id)
            .map(|(_, _, json)| json.as_str())
    }

    fn definition_for(
        &mut self,
        source: u32,
        prepared: &valle_motion::PreparedScene,
        fonts: &Fonts,
    ) -> Result<Option<String>, ProgramPrepareError> {
        if self.oversized_sources.contains(&source) {
            return Ok(None);
        }
        if let Some(index) = self.definitions.iter().position(|entry| entry.0 == source) {
            let entry = self.definitions.remove(index).expect("located definition");
            let id = entry.1.clone();
            self.definitions.push_back(entry);
            return Ok(Some(id));
        }
        // Base64 fonts alone can exceed the wire budget. Do not encode and discard the same
        // immutable font pack on every frame before falling back to ordinary preparation.
        let font_bytes: usize = fonts
            .iter()
            .map(|(_, bytes, _)| bytes.len().div_ceil(3) * 4)
            .sum();
        if font_bytes >= MAX_DEFINITION_BYTES {
            self.oversized_sources.insert(source);
            return Ok(None);
        }
        let json = serde_json::to_string(&Definition {
            artifact: prepared.artifact().clone(),
            fonts: fonts
                .iter()
                .map(|(digest, bytes, generic)| FontDefinition {
                    digest: *digest,
                    bytes: STANDARD.encode(bytes),
                    generic: *generic,
                })
                .collect(),
        })
        .map_err(failure)?;
        if json.len() > MAX_DEFINITION_BYTES {
            self.oversized_sources.insert(source);
            return Ok(None);
        }
        let id = ContentDigest::of_bytes(json.as_bytes()).to_string();
        self.definitions.push_back((source, id.clone(), json));
        while self.definitions.len() > 4
            || self
                .definitions
                .iter()
                .map(|entry| entry.2.len())
                .sum::<usize>()
                > MAX_DEFINITION_BYTES
        {
            self.definitions.pop_front();
        }
        Ok(Some(id))
    }

    pub(super) fn build(
        &mut self,
        key: &str,
        source: u32,
        fonts: &Fonts,
        context: CompiledMotionProgramContext<'_>,
    ) -> Result<BuiltMotionProgram, ProgramPrepareError> {
        if self.oversized_sources.contains(&source)
            || context.prepared.artifact().nodes.len() < MIN_KERNEL_NODES
        {
            return motion::build_compiled_motion_program(context);
        }
        let definition_id = if matches!(self.stage, Some(Stage::Capture(_))) {
            let Some(id) = self.definition_for(source, context.prepared, fonts)? else {
                return motion::build_compiled_motion_program(context);
            };
            Some(id)
        } else {
            None
        };
        let Some(stage) = &mut self.stage else {
            return motion::build_compiled_motion_program(context);
        };
        match stage {
            Stage::Complete(results) => results
                .remove(key)
                .ok_or_else(|| failure("missing Motion kernel result")),
            Stage::Capture(jobs) => {
                let input = motion::resolve_motion_input(&context)?;
                jobs.push(MotionKernelJob {
                    key: key.into(),
                    definition_id: definition_id.expect("capture definition resolved above"),
                    input_json: serde_json::to_string(&input).map_err(failure)?,
                });
                // A capture projection is never returned as a ticket. Empty programs only let
                // the normal traversal discover all clips, including transition endpoints.
                Ok(BuiltMotionProgram {
                    program: DrawProgramBuilder::new(Rect::new(
                        0.0,
                        0.0,
                        f64::from(input.viewport.width()),
                        f64::from(input.viewport.height()),
                    ))
                    .finish()
                    .map_err(failure)?,
                    scene3d_frames: Vec::new(),
                    scene3d_sampling: Vec::new(),
                    layout_boxes: BTreeMap::new(),
                })
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct MotionKernel {
    scene: Option<(String, valle_motion::PreparedScene, Fonts)>,
    fonts: motion::MotionFontCache,
    styles: valle_motion::StyleCache,
    faces: valle_motion::FaceCache,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Metadata {
    scene3d_frames: Vec<valle_motion::SealedScene3DFrame>,
    scene3d_sampling: Vec<valle_motion::emit::Scene3DSampling>,
    layout_boxes: BTreeMap<String, [f32; 4]>,
}

impl MotionKernel {
    pub fn run(
        &mut self,
        definition_id: &str,
        definition_json: &str,
        input_json: &str,
        compact: bool,
    ) -> Result<Vec<u8>, ProgramPrepareError> {
        if definition_json.len() > MAX_DEFINITION_BYTES || input_json.len() > 8 * 1024 * 1024 {
            return Err(failure("Motion kernel input byte budget"));
        }
        if self
            .scene
            .as_ref()
            .is_none_or(|(id, _, _)| id != definition_id)
        {
            if ContentDigest::of_bytes(definition_json.as_bytes()).to_string() != definition_id {
                return Err(failure("Motion definition digest mismatch"));
            }
            let definition: Definition = serde_json::from_str(definition_json).map_err(failure)?;
            let mut fonts = Vec::with_capacity(definition.fonts.len());
            for font in definition.fonts {
                let bytes = STANDARD.decode(&font.bytes).map_err(failure)?;
                if ContentDigest::of_bytes(&bytes) != font.digest {
                    return Err(failure("Motion font digest mismatch"));
                }
                fonts.push((font.digest, Arc::from(bytes), font.generic));
            }
            let scene = valle_motion::prepare_owned_scene(definition.artifact).map_err(failure)?;
            self.scene = Some((definition_id.into(), scene, fonts));
        }
        let (_, scene, dependencies) = self.scene.as_ref().expect("initialized above");
        let input: MotionFrameInput = serde_json::from_str(input_json).map_err(failure)?;
        let fonts = self.fonts.get(dependencies)?;
        let built = valle_draw::program::with_instance_compaction(compact, || {
            motion::build_motion_program(scene, &input, &self.styles, &self.faces, fonts)
        })?;
        let metadata = serde_json::to_vec(&Metadata {
            scene3d_frames: built.scene3d_frames,
            scene3d_sampling: built.scene3d_sampling,
            layout_boxes: built.layout_boxes,
        })
        .map_err(failure)?;
        let program = built.program.packed_bytes().map_err(failure)?;
        let mut bytes = Vec::with_capacity(4 + metadata.len() + program.len());
        bytes.extend_from_slice(
            &u32::try_from(metadata.len())
                .map_err(failure)?
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&metadata);
        bytes.extend_from_slice(&program);
        Ok(bytes)
    }
}

pub(super) fn decode(bytes: &[u8]) -> Result<BuiltMotionProgram, ProgramPrepareError> {
    if bytes.len() < 4 || bytes.len() > 128 * 1024 * 1024 {
        return Err(failure("Motion kernel packet size"));
    }
    let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let end = 4_usize
        .checked_add(length)
        .filter(|end| *end < bytes.len())
        .ok_or_else(|| failure("Motion kernel metadata size"))?;
    let metadata: Metadata = serde_json::from_slice(&bytes[4..end]).map_err(failure)?;
    let program = DrawProgram::from_packed(&bytes[end..]).map_err(failure)?;
    Ok(BuiltMotionProgram {
        program,
        scene3d_frames: metadata.scene3d_frames,
        scene3d_sampling: metadata.scene3d_sampling,
        layout_boxes: metadata.layout_boxes,
    })
}

pub(super) fn failure(reason: impl std::fmt::Display) -> ProgramPrepareError {
    ProgramPrepareError::CompiledInvariant {
        reason: reason.to_string(),
    }
}

pub(crate) fn decode_results(
    mut bytes: &[u8],
) -> Result<BTreeMap<String, BuiltMotionProgram>, ProgramPrepareError> {
    fn field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], ProgramPrepareError> {
        if bytes.len() < 4 {
            return Err(failure("truncated Motion result field"));
        }
        let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        *bytes = &bytes[4..];
        if length > bytes.len() {
            return Err(failure("Motion result field length"));
        }
        let result = &bytes[..length];
        *bytes = &bytes[length..];
        Ok(result)
    }
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(failure("Motion results byte budget"));
    }
    let mut results = BTreeMap::new();
    while !bytes.is_empty() {
        if results.len() >= 128 {
            return Err(failure("Motion results count budget"));
        }
        let key = std::str::from_utf8(field(&mut bytes)?).map_err(failure)?;
        if key.len() > 4096 || key.is_empty() {
            return Err(failure("Motion result key"));
        }
        let value = decode(field(&mut bytes)?)?;
        if results.insert(key.into(), value).is_some() {
            return Err(failure("duplicate Motion result key"));
        }
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::Extent2d;
    use valle_timeline::FrameRate;

    fn fixture() -> (Definition, MotionFrameInput) {
        let artifact = valle_compiler::motion::compile_motion(r##"
export const composition = { width: 320, height: 180, duration: 1 };
export const controls = { props: { alpha: number({ default: 1, min: 0, max: 1 }) } };
export default function Card(props) { const time = useCurrentTime(); const frame = Math.floor(time.seconds * 30);
  return <Scene style={{ width: 320, height: 180, perspective: 800 }}>
    <View key="card" style={{ position: "absolute", left: frame * 2, width: 140, height: 80,
      transform: `rotateY(${frame}deg)`, transformStyle: "preserve-3d", opacity: props.alpha,
      backgroundColor: "#123456", borderRadius: 8 }}>
      <Text key="label" style={{ fontSize: 24 }}>VALLE</Text>
    </View>
  </Scene>;
}
"##).unwrap().artifact;
        let bytes = include_bytes!("../../../../assets/fonts/katex/KaTeX_Main-Regular.ttf");
        let definition = Definition {
            artifact,
            fonts: vec![FontDefinition {
                digest: ContentDigest::of_bytes(bytes),
                bytes: STANDARD.encode(bytes),
                generic: true,
            }],
        };
        let input = MotionFrameInput {
            props: BTreeMap::from([("alpha".into(), valle_motion::MotionValue::Number(0.75))]),
            motion_context: valle_motion::motion_context_at_frame(
                0,
                30,
                FrameRate::new(30, 1).unwrap(),
            )
            .unwrap(),
            raster_scale: valle_motion::emit::RasterDensity::default(),
            viewport: Extent2d::new(320, 180).unwrap(),
            source_clip: Some(Rect::new(0.0, 0.0, 320.0, 180.0)),
            assets: BTreeMap::new(),
            model_resources: BTreeMap::new(),
        };
        (definition, input)
    }

    #[test]
    fn oversized_font_packs_do_not_create_or_retry_kernel_definitions() {
        let (definition, _) = fixture();
        let scene = valle_motion::prepare_owned_scene(definition.artifact).unwrap();
        let bytes: Arc<[u8]> = vec![0; 26 * 1024 * 1024].into();
        let font = (ContentDigest::of_bytes(&bytes), bytes, true);
        let mut preparation = MotionPreparation::default();
        assert!(
            preparation
                .definition_for(0, &scene, &vec![font.clone(), font])
                .unwrap()
                .is_none()
        );
        assert!(preparation.definitions.is_empty());
        // A source is immutable for this preparation owner. Once rejected it cannot repeatedly
        // allocate and hash another oversized definition while scrubbing later frames.
        assert!(
            preparation
                .definition_for(0, &scene, &vec![])
                .unwrap()
                .is_none()
        );
        let id = preparation
            .definition_for(1, &scene, &vec![])
            .unwrap()
            .unwrap();
        assert!(preparation.definition(&id).is_some());
        assert_eq!(
            preparation.definition_for(1, &scene, &vec![]).unwrap(),
            Some(id)
        );
    }

    #[test]
    fn independent_motion_kernels_preserve_fonts_props_and_random_frame_order() {
        let (definition, mut input) = fixture();
        let json = serde_json::to_string(&definition).unwrap();
        let id = ContentDigest::of_bytes(json.as_bytes()).to_string();
        let scene = valle_motion::prepare_owned_scene(definition.artifact).unwrap();
        let dependencies = definition
            .fonts
            .iter()
            .map(|font| {
                (
                    font.digest,
                    Arc::from(STANDARD.decode(&font.bytes).unwrap()),
                    font.generic,
                )
            })
            .collect::<Vec<_>>();
        let mut fonts = motion::MotionFontCache::default();
        let styles = valle_motion::StyleCache::default();
        let faces = valle_motion::FaceCache::default();
        let mut kernels = [MotionKernel::default(), MotionKernel::default()];
        for (index, frame) in [23, 3, 23, 0, 29, 3].into_iter().enumerate() {
            input.motion_context =
                valle_motion::motion_context_at_frame(frame, 30, FrameRate::new(30, 1).unwrap())
                    .unwrap();
            let expected = motion::build_motion_program(
                &scene,
                &input,
                &styles,
                &faces,
                fonts.get(&dependencies).unwrap(),
            )
            .unwrap();
            let packet = kernels[index % 2]
                .run(&id, &json, &serde_json::to_string(&input).unwrap(), false)
                .unwrap();
            let actual = decode(&packet).unwrap();
            assert_eq!(
                actual.program.packed_bytes().unwrap(),
                expected.program.packed_bytes().unwrap()
            );
            assert_eq!(actual.layout_boxes, expected.layout_boxes);
            assert_eq!(
                serde_json::to_vec(&actual.scene3d_frames).unwrap(),
                serde_json::to_vec(&expected.scene3d_frames).unwrap()
            );
        }
        assert!(
            kernels[0]
                .run(&id, "", &serde_json::to_string(&input).unwrap(), false)
                .is_ok()
        );
    }

    #[test]
    fn kernel_rejects_definition_drift_bad_fonts_and_truncated_or_duplicate_results() {
        let (mut definition, input) = fixture();
        let json = serde_json::to_string(&definition).unwrap();
        let id = ContentDigest::of_bytes(json.as_bytes()).to_string();
        let input = serde_json::to_string(&input).unwrap();
        let mut kernel = MotionKernel::default();
        assert!(kernel.run(&id, "{}", &input, false).is_err());
        let packet = kernel.run(&id, &json, &input, false).unwrap();
        for bytes in [&packet[..3], &packet[..packet.len() - 1]] {
            assert!(decode(bytes).is_err());
        }
        let mut records = Vec::new();
        for _ in 0..2 {
            records.extend_from_slice(&1_u32.to_le_bytes());
            records.push(b'x');
            records.extend_from_slice(&(packet.len() as u32).to_le_bytes());
            records.extend_from_slice(&packet);
        }
        assert!(decode_results(&records).is_err());
        assert!(decode_results(&[1, 0, 0, 0]).is_err());
        definition.fonts[0].digest = ContentDigest::of_bytes(b"wrong font");
        let json = serde_json::to_string(&definition).unwrap();
        let id = ContentDigest::of_bytes(json.as_bytes()).to_string();
        assert!(
            kernel
                .run(&id, &json, &input, false)
                .unwrap_err()
                .to_string()
                .contains("font digest mismatch")
        );
    }
}
