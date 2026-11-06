//! Sample real scene frames and compare measured boxes and lines with explicit expectations.

use std::path::Path;
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use valle_engine::front::Inputs;
use valle_engine::image::Image;
use valle_engine::scene::{Frame, Scene, Video};
use valle_engine::time::Tick;
use valle_engine::timeline;
use valle_native::media::{self, Decoding};

use crate::case::{Case, Layout, Line, Lines};

pub struct Sample {
    pub at: Tick,
    pub scene: Scene,
    pub frame: Frame,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Measurements {
    pub layout: Vec<Layout>,
    pub lines: Vec<Lines>,
}

impl Sample {
    pub fn number(&self) -> i64 {
        self.scene.composition().fps.frame_at(self.at)
    }

    pub fn size(&self) -> [u32; 2] {
        let c = self.scene.composition();
        [c.width, c.height]
    }
}

pub fn load(root: &Path, file: &Path, at: Tick) -> Result<Sample, String> {
    crate::fonts::prepare()?;
    let relative = file
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    // The test host mounts the repository at `/`, as the browser capture server does.
    // Shared `/tests/resources/...` paths stay valid when a case moves; `./source.svg`
    // still resolves beside the scene through the production loader.
    let entry = format!("/{relative}");
    let locate = |path: &str| root.join(path.trim_start_matches('/'));
    let resources = root.join("tests/resources");
    let fonts = Rc::new(move |font: &valle_engine::text::BuiltIn| {
        let bytes = std::fs::read(resources.join(font.file)).map_err(|e| e.to_string())?;
        if crate::browser::sha256(&bytes) != font.sha256 {
            return Err(format!("{} does not match the pinned font", font.file));
        }
        Ok(valle_engine::text::Blob::from(bytes))
    });
    let lowered = if entry.ends_with(".timeline.json") {
        let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
        let doc = valle_timeline::parse(&text).map_err(|e| format!("{entry}: {e}"))?;
        Some(timeline::lower(
            &entry,
            &doc,
            &timeline::Host {
                read: &|path| std::fs::read_to_string(locate(path)).ok(),
                length: &|path| media::length(&locate(path)).ok().map(Tick),
            },
        )?)
    } else {
        None
    };
    let (entry, inputs) = match &lowered {
        Some(l) => (l.entry.as_str(), l.inputs.clone()),
        None => (entry.as_str(), Inputs::default()),
    };
    let read = |path: &str| match lowered.as_ref().and_then(|l| l.files.get(path)) {
        Some(source) => Some(source.as_bytes().to_vec()),
        None => std::fs::read(locate(path)).ok(),
    };
    let mut scene = Scene::load(entry, &read, &inputs, fonts)?;
    for (name, file) in scene.video_files(&inputs) {
        let reader = media::reader(&locate(&file), Some(Decoding::Software))?;
        scene.set_video(&name, Box::new(Decoded(reader)))?;
    }
    if at >= scene.composition().duration {
        return Err(format!(
            "at={}s is outside the scene duration",
            at.seconds_text()
        ));
    }
    let frame = scene.frame(at)?;
    Ok(Sample { at, scene, frame })
}

pub fn measure(frame: &Frame) -> Measurements {
    let mut result = Measurements {
        layout: Vec::new(),
        lines: Vec::new(),
    };
    for (i, node) in frame.template.nodes.iter().enumerate() {
        let Some(key) = &node.path else { continue };
        let boxes = frame.layout.boxes[i];
        result.layout.push(Layout {
            key: key.to_string(),
            border: boxes.border.into(),
            padding: boxes.padding.into(),
            content: boxes.content.into(),
        });
        if let Some(lines) = &frame.layout.lines[i] {
            result.lines.push(Lines {
                key: key.to_string(),
                text: lines
                    .text
                    .iter()
                    .map(|r| lines.string[r.clone()].trim_end_matches('\n').to_string())
                    .collect(),
                geometry: lines
                    .boxes
                    .iter()
                    .enumerate()
                    .map(|(line, b)| Line {
                        x: boxes.content.x
                            + lines
                                .runs
                                .iter()
                                .find(|r| r.line as usize == line)
                                .map_or(0.0, |r| r.x[0]),
                        baseline: boxes.content.y + lines.baselines[line],
                        height: b[1],
                    })
                    .collect(),
            });
        }
    }
    result
}

pub fn check(root: &Path, dir: &Path, case: &Case) -> Result<usize, String> {
    let sample = load(root, &case.input(dir), case.times()?[0])?;
    layout(&sample.frame, case)
}

/// Fixed software decoding keeps the video input independent of the GPU backend.
struct Decoded(Box<dyn media::Read>);

impl Video for Decoded {
    fn size(&self) -> [u32; 2] {
        self.0.size()
    }

    fn frame(&mut self, at: Tick) -> Result<Image, String> {
        Ok(Image {
            size: self.0.size(),
            pixels: self.0.frame(at.0)?.into(),
        })
    }
}

pub fn layout(frame: &Frame, case: &Case) -> Result<usize, String> {
    measure(frame).check(case)
}

impl Measurements {
    pub fn check(&self, case: &Case) -> Result<usize, String> {
        let mut failures = Vec::new();
        let mut checks = 0;
        for expected in &case.layout {
            let nodes: Vec<_> = self
                .layout
                .iter()
                .filter(|m| m.key == expected.key)
                .collect();
            let actual = match nodes.as_slice() {
                [one] => one,
                [] => {
                    failures.push(format!(
                        "key {:?}: no node in the sampled frame",
                        expected.key
                    ));
                    continue;
                }
                _ => {
                    failures.push(format!(
                        "key {:?}: more than one node in the sampled frame",
                        expected.key
                    ));
                    continue;
                }
            };
            for (name, want, got) in [
                ("border", &expected.border, &actual.border),
                ("padding", &expected.padding, &actual.padding),
                ("content", &expected.content, &actual.content),
            ] {
                want.check(
                    got.rect(),
                    &format!("{}.{name}", expected.key),
                    &mut failures,
                );
                checks += 4;
            }
        }
        for expected in &case.lines {
            let found: Vec<_> = self
                .lines
                .iter()
                .filter(|l| l.key == expected.key)
                .collect();
            if !matches!(found.as_slice(), [actual] if actual.text == expected.text) {
                failures.push(format!(
                    "{}.lines: expected {:?}, actual {:?}",
                    expected.key,
                    expected.text,
                    found.iter().map(|l| &l.text).collect::<Vec<_>>()
                ));
            }
            checks += 1;
            if let [actual] = found.as_slice() {
                if actual.geometry.len() != expected.geometry.len() {
                    failures.push(format!("{}.geometry: wrong number of lines", expected.key));
                    continue;
                }
                for (i, (want, got)) in expected.geometry.iter().zip(&actual.geometry).enumerate() {
                    for (field, want, got) in [
                        ("x", want.x, got.x),
                        ("baseline", want.baseline, got.baseline),
                        ("height", want.height, got.height),
                    ] {
                        // Two Chrome layout units; never enough to hide a one-pixel displacement.
                        if !got.is_finite() || (got - want).abs() > 2.0 / 64.0 {
                            failures.push(format!(
                                "{}.line[{i}].{field}: expected {want}, actual {got}",
                                expected.key
                            ));
                        }
                        checks += 1;
                    }
                }
            }
        }
        if failures.is_empty() {
            Ok(checks)
        } else {
            Err(format!(
                "{}\n{}\n{}",
                case.assertion,
                case.spec,
                failures.join("\n")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_resources_survive_case_relocation() {
        crate::with_stack(|| {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            let moved = root.join(format!(
                "target/test-results/resource-relocation-{}",
                std::process::id()
            ));
            for name in [
                "css/images/object-fit",
                "css/images/object-position",
                "tailwind/filters",
                "timeline/clips",
                "timeline/transitions",
                "timeline/captions",
                "timeline/effects",
                "timeline/look",
                "timeline/nested",
            ] {
                let dir = root.join("tests/cases").join(name);
                let case = Case::read(&dir.join("case.json")).unwrap();
                let input = case.input(&dir);
                let times = case.times().unwrap();
                let mut boundaries = vec![times[0], *times.last().unwrap()];
                boundaries.dedup();
                for at in boundaries {
                    let original = load(&root, &input, at).unwrap();
                    for depth in ["flat", "deeper/another/level"] {
                        let dir = moved.join(name).join(depth);
                        std::fs::create_dir_all(&dir).unwrap();
                        let relocated = dir.join(input.file_name().unwrap());
                        std::fs::copy(&input, &relocated).unwrap();
                        // Copy only the entry: every shared dependency must resolve from
                        // its stable path, including nested components and video decoding.
                        let sample = load(&root, &relocated, at).unwrap();
                        assert_eq!(original.frame.items, sample.frame.items, "{name}");
                        assert_eq!(original.frame.pictures, sample.frame.pictures, "{name}");
                        assert_eq!(
                            serde_json::to_value(measure(&original.frame)).unwrap(),
                            serde_json::to_value(measure(&sample.frame)).unwrap(),
                            "{name}"
                        );
                        let images = original.scene.images();
                        let relocated = sample.scene.images();
                        assert_eq!(images.len(), relocated.len(), "{name}");
                        for (a, b) in images.iter().zip(relocated) {
                            assert_eq!(a.size, b.size, "{name}");
                            assert!(a.pixels == b.pixels, "{name}: resource pixels changed");
                        }
                    }
                }
            }
            std::fs::remove_dir_all(moved).unwrap();
        });
    }
}
