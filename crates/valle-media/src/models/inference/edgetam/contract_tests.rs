use super::*;
use crate::models::spec::embedded_release_manifest;
use serde_json::json;

const PROTOCOL: &str = include_str!("../../../../tests/fixtures/edgetam/prompt_protocol.v1.json");

struct Fixture {
    root: tempfile::TempDir,
    manifest: ModelManifest,
    route: Route,
    artifact: Artifact,
}

impl Fixture {
    fn new() -> Self {
        let manifest = embedded_release_manifest("edgetam", "1.0.0").unwrap();
        let route = manifest
            .routes
            .iter()
            .find(|r| r.backend == Backend::OnnxCpu)
            .unwrap()
            .clone();
        let artifact = manifest
            .artifacts
            .iter()
            .find(|a| a.id == route.artifact)
            .unwrap()
            .clone();
        let root = tempfile::tempdir().unwrap();
        // Contract validation checks the package layout, without opening model weights.
        for file in REQUIRED_FILES {
            std::fs::write(root.path().join(file), b"contract fixture").unwrap();
        }
        std::fs::write(root.path().join("prompt_protocol.v1.json"), PROTOCOL).unwrap();
        Self {
            root,
            manifest,
            route,
            artifact,
        }
    }

    fn validate(&self) -> Result<()> {
        validate_contract(
            &self.manifest,
            &self.route,
            &self.artifact,
            self.root.path(),
        )
    }

    fn rejects(&self, reason: &str) {
        let error = self.validate().unwrap_err();
        assert!(
            format!("{error:#}").contains(reason),
            "expected {reason:?}: {error:#}"
        );
    }
}

#[test]
fn published_onnx_release_and_prompt_fixture_satisfy_the_host_contract() {
    Fixture::new().validate().unwrap();
}

#[test]
fn incompatible_release_and_adapter_are_rejected() {
    let cases: [(&str, fn(&mut ModelManifest)); 5] = [
        ("not EdgeTAM", |m| m.model.id = "other-model".into()),
        ("unsupported EdgeTAM release", |m| {
            m.model.version = "2.0.0".into()
        }),
        ("unsupported task", |m| {
            m.model.task = "image-matting".into()
        }),
        ("unsupported EdgeTAM adapter", |m| {
            m.contract.adapter = "other-adapter".into()
        }),
        ("unsupported EdgeTAM adapter", |m| m.contract.version = 2),
    ];
    for (reason, mutate) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture.manifest);
        fixture.rejects(reason);
    }
}

#[test]
fn tensors_must_keep_their_order_type_layout_shape_and_range() {
    let cases: [fn(&mut ModelManifest); 7] = [
        |m| m.contract.inputs[0].name = "frame".into(),
        |m| m.contract.inputs[1].dtype = "i32".into(),
        |m| m.contract.inputs[2].layout = "pn".into(),
        |m| m.contract.inputs[0].shape[2] = Dimension::Fixed(512),
        |m| m.contract.inputs[1].range = Some([0.0, 1.0]),
        |m| m.contract.inputs.swap(1, 2),
        |m| m.contract.outputs[0].shape[1] = Dimension::Fixed(3),
    ];
    for mutate in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture.manifest);
        fixture.rejects("contract drifted");
    }
    let mut fixture = Fixture::new();
    fixture.manifest.contract.inputs.pop();
    fixture.rejects("must expose image/coords/labels");
}

#[test]
fn preprocessing_and_stateful_host_operations_cannot_drift() {
    let cases: [(&str, fn(&mut ModelManifest)); 6] = [
        ("preprocess contract is invalid", |m| {
            m.contract.preprocess = json!({})
        }),
        ("preprocessing contract drifted", |m| {
            m.contract.preprocess["color_space"] = json!("bgr")
        }),
        ("preprocessing contract drifted", |m| {
            m.contract.preprocess["canvas"] = json!([512, 1024])
        }),
        ("postprocess contract is invalid", |m| {
            m.contract.postprocess = json!({})
        }),
        ("must remain stateful", |m| {
            m.contract.postprocess["stateful"] = json!(false)
        }),
        ("host contract drifted", |m| {
            m.contract.postprocess["host_contract"]
                .as_array_mut()
                .unwrap()
                .swap(0, 1)
        }),
    ];
    for (reason, mutate) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture.manifest);
        fixture.rejects(reason);
    }
}

#[test]
fn route_runtime_and_memory_options_are_validated_before_loading() {
    let cases: [(&str, fn(&mut Route)); 6] = [
        ("route and artifact disagree", |r| {
            r.artifact = "other-artifact".into()
        }),
        ("onnx-cpu route", |r| r.backend = Backend::Coreml),
        ("ONNX Runtime 1.28.0", |r| {
            r.requirements.minimum_runtime = None
        }),
        ("route options drifted", |r| {
            r.options["bounded_prefetch"] = json!(2)
        }),
        ("route options drifted", |r| {
            r.options["borrow_inputs"] = json!(false)
        }),
        ("route options drifted", |r| {
            r.options["cache_full_memory_constants"] = json!(false)
        }),
    ];
    for (reason, mutate) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture.route);
        fixture.rejects(reason);
    }
}

#[test]
fn artifact_format_and_component_order_are_pinned() {
    let cases: [(&str, fn(&mut Artifact)); 6] = [
        ("artifact identity drifted", |a| {
            a.format = "coreml-package".into()
        }),
        ("artifact identity drifted", |a| a.precision = "fp16".into()),
        ("artifact identity drifted", |a| {
            a.entrypoint = "decoder_p3.onnx".into()
        }),
        ("metadata is invalid", |a| a.metadata = json!({})),
        ("component contract drifted", |a| {
            a.metadata["opset"] = json!(18)
        }),
        ("component contract drifted", |a| {
            a.metadata["components"].as_array_mut().unwrap().swap(0, 1)
        }),
    ];
    for (reason, mutate) in cases {
        let mut fixture = Fixture::new();
        mutate(&mut fixture.artifact);
        fixture.rejects(reason);
    }
}

#[test]
fn every_required_component_must_be_declared_and_present_as_a_file() {
    for required in REQUIRED_FILES {
        let mut undeclared = Fixture::new();
        undeclared.artifact.files.retain(|f| f.path != required);
        undeclared.rejects(&format!("does not declare {required}"));
        let missing = Fixture::new();
        std::fs::remove_file(missing.root.path().join(required)).unwrap();
        missing.rejects("artifact file is missing");
        std::fs::create_dir(missing.root.path().join(required)).unwrap();
        missing.rejects("artifact file is missing");
    }
}

#[test]
fn malformed_or_semantically_incompatible_prompt_protocol_is_rejected() {
    let cases: [(&str, fn(&mut serde_json::Value)); 8] = [
        ("protocol identity drifted", |p| {
            p["schema_version"] = json!(2)
        }),
        ("protocol identity drifted", |p| {
            p["decoder"] = json!("decoder_p1")
        }),
        ("protocol identity drifted", |p| p["seed_frame"] = json!(1)),
        ("protocol identity drifted", |p| {
            p["coordinate_space"] = json!("normalized")
        }),
        ("protocol identity drifted", |p| p["point_count"] = json!(2)),
        ("prompt labels drifted", |p| {
            p["labels"] = json!({"0": "positive", "1": "negative"})
        }),
        ("prompt selection rules drifted", |p| {
            p["selection_rule"].as_array_mut().unwrap().pop();
        }),
        ("prompt selection rules drifted", |p| {
            p["selection_rule"][2] = json!("Use prompts from later frames.")
        }),
    ];
    for (reason, mutate) in cases {
        let fixture = Fixture::new();
        let mut protocol = serde_json::from_str(PROTOCOL).unwrap();
        mutate(&mut protocol);
        std::fs::write(
            fixture.root.path().join("prompt_protocol.v1.json"),
            serde_json::to_vec(&protocol).unwrap(),
        )
        .unwrap();
        fixture.rejects(reason);
    }
    let fixture = Fixture::new();
    let path = fixture.root.path().join("prompt_protocol.v1.json");
    std::fs::write(&path, b"{broken").unwrap();
    fixture.rejects("invalid EdgeTAM prompt protocol");
    std::fs::remove_file(&path).unwrap();
    assert!(
        format!("{:#}", validate_prompt_protocol(&path).unwrap_err())
            .contains("failed to read EdgeTAM prompt protocol")
    );
}
