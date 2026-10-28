//! Versioned kernel results. A receiver advertises only templates it currently owns. That is
//! a bandwidth hint, not a lifetime promise: an in-flight result may outlive an LRU entry.
//! A missing reference explicitly requests authoritative local rebuilding, never stale pixels.
use super::{Metadata, failure};
use crate::{
    prepare::motion::{BuiltMotionProgram, CachedDrawProgram, ProgramPrepareError},
    resource::ContentDigest,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use valle_draw::program::{PaintFrame, PaintTemplate};

const MAGIC: &[u8; 8] = b"VLMOTN02";
const MAX_PACKET_BYTES: usize = 128 * 1024 * 1024;
pub(super) const MAX_TEMPLATES: usize = 8;
// Serialized-size accounting plus DrawProgram's independent decoded arena/count limits. This
// is not an RSS bound: Rust arenas and metadata maps have additional allocation overhead.
const MAX_TEMPLATE_BYTES: usize = 16 * 1024 * 1024;

type Boxes = Arc<BTreeMap<String, [f32; 4]>>;

struct EncodedTemplate {
    template: Arc<PaintTemplate>,
    boxes: Boxes,
    bytes: Vec<u8>,
    id: ContentDigest,
}

#[derive(Default)]
pub(super) struct Encoder {
    // One resident variant per worker. A seek may replace it, but cannot change its identity.
    template: Option<EncodedTemplate>,
}

impl Encoder {
    pub(super) fn encode(
        &mut self,
        built: BuiltMotionProgram,
        known: &[ContentDigest],
    ) -> Result<Vec<u8>, ProgramPrepareError> {
        let mut bytes = MAGIC.to_vec();
        if let Some(frame) = built.program.paint_frame.as_ref()
            && built.scene3d_frames.is_empty()
            && built.scene3d_sampling.is_empty()
        {
            if self.template.as_ref().is_none_or(|entry| {
                !Arc::ptr_eq(&entry.template, frame.template())
                    || !Arc::ptr_eq(&entry.boxes, &built.layout_boxes)
            }) {
                let metadata = serde_json::to_vec(&Metadata {
                    scene3d_frames: Vec::new(),
                    scene3d_sampling: Vec::new(),
                    layout_boxes: built.layout_boxes.clone(),
                })
                .map_err(failure)?;
                let mut payload = Vec::new();
                append_field(&mut payload, &metadata)?;
                payload.extend_from_slice(frame.template().packed().map_err(failure)?);
                self.template = Some(EncodedTemplate {
                    template: frame.template().clone(),
                    boxes: built.layout_boxes.clone(),
                    id: ContentDigest::of_bytes(&payload),
                    bytes: payload,
                });
            }
            let template = self.template.as_ref().expect("initialized above");
            let reference = known.contains(&template.id);
            bytes.push(if reference { 2 } else { 1 });
            bytes.extend_from_slice(template.id.as_bytes());
            if !reference {
                append_field(&mut bytes, &template.bytes)?;
            }
            bytes.extend_from_slice(&frame.parameter_bytes());
        } else {
            bytes.push(0);
            let metadata = serde_json::to_vec(&Metadata {
                scene3d_frames: built.scene3d_frames,
                scene3d_sampling: built.scene3d_sampling,
                layout_boxes: built.layout_boxes,
            })
            .map_err(failure)?;
            append_field(&mut bytes, &metadata)?;
            bytes.extend_from_slice(built.program.packed().map_err(failure)?);
        }
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(failure("Motion kernel packet byte budget"));
        }
        Ok(bytes)
    }
}

pub(crate) enum ResultProgram {
    Built(BuiltMotionProgram),
    /// No leases or previous-frame dependence. The current scene/input is rebuilt locally.
    Rebuild,
}

struct AdmittedTemplate {
    id: ContentDigest,
    template: Arc<PaintTemplate>,
    boxes: Boxes,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct Receiver {
    templates: VecDeque<AdmittedTemplate>,
}

impl Receiver {
    pub(super) fn known_templates(&self) -> Vec<ContentDigest> {
        self.templates.iter().map(|entry| entry.id).collect()
    }

    pub(super) fn decode(&mut self, bytes: &[u8]) -> Result<ResultProgram, ProgramPrepareError> {
        if bytes.len() < 9 || bytes.len() > MAX_PACKET_BYTES || &bytes[..8] != MAGIC {
            return Err(failure("Motion kernel packet header or size"));
        }
        let mut payload = &bytes[9..];
        match bytes[8] {
            0 => {
                let metadata: Metadata =
                    serde_json::from_slice(field(&mut payload)?).map_err(failure)?;
                let program = CachedDrawProgram::from_packed(payload).map_err(failure)?;
                Ok(ResultProgram::Built(BuiltMotionProgram {
                    program: Arc::new(program),
                    scene3d_frames: metadata.scene3d_frames,
                    scene3d_sampling: metadata.scene3d_sampling,
                    layout_boxes: metadata.layout_boxes,
                }))
            }
            tag @ (1 | 2) => {
                if payload.len() < 32 {
                    return Err(failure("Motion template digest size"));
                }
                let id = ContentDigest::from_bytes(payload[..32].try_into().unwrap());
                payload = &payload[32..];
                let inline = if tag == 1 {
                    let bytes = field(&mut payload)?;
                    if ContentDigest::of_bytes(bytes) != id {
                        return Err(failure("Motion template digest mismatch"));
                    }
                    // Several cold jobs may have been captured before the first reply arrived.
                    // Verify each packet's identity, but admit equal template bytes only once.
                    if let Some(index) = self.templates.iter().position(|entry| entry.id == id) {
                        self.templates.remove(index)
                    } else {
                        let mut content = bytes;
                        let metadata: Metadata =
                            serde_json::from_slice(field(&mut content)?).map_err(failure)?;
                        if !metadata.scene3d_frames.is_empty()
                            || !metadata.scene3d_sampling.is_empty()
                        {
                            return Err(failure("paint template cannot carry Scene3D producers"));
                        }
                        Some(AdmittedTemplate {
                            id,
                            template: Arc::new(
                                PaintTemplate::from_packed(content).map_err(failure)?,
                            ),
                            boxes: metadata.layout_boxes,
                            bytes: bytes.len(),
                        })
                    }
                } else {
                    None
                };
                // Validate the parameter envelope even when the referenced template was evicted.
                if payload.len() > 65_536 * 16 || !payload.len().is_multiple_of(16) {
                    return Err(failure("Motion paint parameter byte budget"));
                }
                let entry = inline.or_else(|| {
                    self.templates
                        .iter()
                        .position(|entry| entry.id == id)
                        .and_then(|index| self.templates.remove(index))
                });
                let Some(entry) = entry else {
                    return Ok(ResultProgram::Rebuild);
                };
                let frame = PaintFrame::from_parameters(entry.template.clone(), payload)
                    .map_err(failure)?;
                let built = BuiltMotionProgram {
                    program: Arc::new(CachedDrawProgram::from_paint_frame(frame)),
                    scene3d_frames: Vec::new(),
                    scene3d_sampling: Vec::new(),
                    layout_boxes: entry.boxes.clone(),
                };
                if entry.bytes <= MAX_TEMPLATE_BYTES {
                    self.templates.retain(|cached| cached.id != id);
                    while self.templates.len() >= MAX_TEMPLATES
                        || self
                            .templates
                            .iter()
                            .map(|cached| cached.bytes)
                            .sum::<usize>()
                            + entry.bytes
                            > MAX_TEMPLATE_BYTES
                    {
                        self.templates.pop_front();
                    }
                    self.templates.push_back(entry);
                }
                Ok(ResultProgram::Built(built))
            }
            _ => Err(failure("Motion kernel packet kind")),
        }
    }

    pub(crate) fn decode_results(
        &mut self,
        mut bytes: &[u8],
    ) -> Result<BTreeMap<String, ResultProgram>, ProgramPrepareError> {
        if bytes.len() > MAX_PACKET_BYTES {
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
            if results.contains_key(key) {
                return Err(failure("duplicate Motion result key"));
            }
            let value = self.decode(field(&mut bytes)?)?;
            results.insert(key.into(), value);
        }
        Ok(results)
    }
}

fn append_field(bytes: &mut Vec<u8>, value: &[u8]) -> Result<(), ProgramPrepareError> {
    bytes.extend_from_slice(&u32::try_from(value.len()).map_err(failure)?.to_le_bytes());
    bytes.extend_from_slice(value);
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use valle_draw::{Rect, program::*};

    fn built(width: f64, color: LinearColor, boxes: Boxes) -> BuiltMotionProgram {
        let mut b = DrawProgramBuilder::new(Rect::new(0., 0., width, 64.));
        let paint = b.push_paint(Paint::Solid(color));
        let path = b.push_path(PathData {
            verbs: vec![
                PathVerb::MoveTo,
                PathVerb::LineTo,
                PathVerb::LineTo,
                PathVerb::Close,
            ],
            points: vec![[0., 0.], [width, 0.], [width, 64.]],
        });
        let node = b.push_node(Node::Path(PathNode {
            path,
            fill_rule: FillRule::NonZero,
            fill: Some(paint),
            stroke: None,
        }));
        b.add_root(node);
        let template = Arc::new(
            PaintTemplate::new(
                b.finish().unwrap(),
                vec![PaintParameter { paint, slot: 0 }],
                1,
            )
            .unwrap(),
        );
        BuiltMotionProgram {
            program: Arc::new(CachedDrawProgram::from_paint_frame(
                template.bind_frame(vec![color]).unwrap(),
            )),
            layout_boxes: boxes,
            scene3d_frames: vec![],
            scene3d_sampling: vec![],
        }
    }

    fn unpack(value: ResultProgram) -> BuiltMotionProgram {
        match value {
            ResultProgram::Built(built) => built,
            ResultProgram::Rebuild => panic!("unexpected miss"),
        }
    }

    #[test]
    fn cold_seek_worker_replacement_and_out_of_order_parameters_are_exact() {
        let mut receiver = Receiver::default();
        let mut encoder = Encoder::default();
        let boxes = Arc::new(BTreeMap::from([("box".into(), [0., 0., 64., 64.])]));
        let mut packets = Vec::new();
        for (index, color) in [
            LinearColor::new(4., -0.1, 0.2, 0.5),
            LinearColor::new(0., 0., 0., 0.),
            LinearColor::new(-0., 0.2, 0., 1.),
        ]
        .into_iter()
        .enumerate()
        {
            let frame = built(64., color, boxes.clone());
            let expected = frame.program.packed_bytes().unwrap();
            let packet = encoder.encode(frame, &receiver.known_templates()).unwrap();
            if index == 0 {
                assert_eq!(packet[8], 1);
                let first = unpack(receiver.decode(&packet).unwrap());
                let duplicate = unpack(receiver.decode(&packet).unwrap());
                assert_eq!(first.program.packed_bytes().unwrap(), expected);
                assert!(Arc::ptr_eq(&first.layout_boxes, &duplicate.layout_boxes));
                assert!(Arc::ptr_eq(
                    first.program.paint_frame.as_ref().unwrap().template(),
                    duplicate.program.paint_frame.as_ref().unwrap().template()
                ));
                // No sender-side assumption that a particular worker supplied the template.
                encoder = Encoder::default();
            } else {
                assert_eq!(packet[8], 2);
                assert_eq!(packet.len(), 8 + 1 + 32 + 16);
            }
            packets.push((packet, expected));
        }
        for index in [2, 1, 2, 0, 1] {
            let (packet, expected) = &packets[index];
            let actual = unpack(receiver.decode(packet).unwrap());
            assert_eq!(&actual.program.packed_bytes().unwrap(), expected);
            assert_eq!(actual.layout_boxes, boxes);
        }
        // Layout metadata is part of the template namespace too.
        let changed = Arc::new(BTreeMap::from([("box".into(), [1., 0., 64., 64.])]));
        let packet = encoder
            .encode(
                built(64., LinearColor::new(1., 0., 0., 1.), changed),
                &receiver.known_templates(),
            )
            .unwrap();
        assert_eq!(packet[8], 1);
    }

    #[test]
    fn eviction_and_cancellation_do_not_create_leases_or_history_dependent_results() {
        let mut receiver = Receiver::default();
        let mut encoder = Encoder::default();
        let color = LinearColor::new(1., 0., 0., 1.);
        let packet = encoder
            .encode(built(64., color, Default::default()), &[])
            .unwrap();
        unpack(receiver.decode(&packet).unwrap());
        let pending = encoder
            .encode(
                built(64., color, Default::default()),
                &receiver.known_templates(),
            )
            .unwrap();
        for i in 0..MAX_TEMPLATES {
            let packet = encoder
                .encode(built(65. + i as f64, color, Default::default()), &[])
                .unwrap();
            unpack(receiver.decode(&packet).unwrap());
        }
        assert_eq!(receiver.templates.len(), MAX_TEMPLATES);
        assert!(matches!(
            receiver.decode(&pending).unwrap(),
            ResultProgram::Rebuild
        ));
        // A new receiver after cancellation/restart likewise rebuilds, then requests an inline
        // template on the next job. No acknowledgement, retained pending lease or resend needed.
        assert!(matches!(
            Receiver::default().decode(&pending).unwrap(),
            ResultProgram::Rebuild
        ));
        let restored = encoder
            .encode(
                built(64., color, Default::default()),
                &receiver.known_templates(),
            )
            .unwrap();
        assert_eq!(restored[8], 1);
        assert_eq!(
            unpack(receiver.decode(&restored).unwrap())
                .program
                .packed_bytes()
                .unwrap(),
            built(64., color, Default::default())
                .program
                .packed_bytes()
                .unwrap()
        );
    }

    #[test]
    fn admission_rejects_corrupt_template_and_parameter_payloads() {
        let mut encoder = Encoder::default();
        let packet = encoder
            .encode(
                built(64., LinearColor::new(1., 0., 0., 1.), Default::default()),
                &[],
            )
            .unwrap();
        for end in [0, 8, 9, 40, packet.len() - 1] {
            assert!(Receiver::default().decode(&packet[..end]).is_err());
        }
        let mut wrong = packet.clone();
        wrong[41] ^= 1;
        assert!(Receiver::default().decode(&wrong).is_err());
        let mut wrong = packet.clone();
        wrong[9] ^= 1;
        assert!(Receiver::default().decode(&wrong).is_err());
        let mut wrong = packet.clone();
        let offset = wrong.len() - 16;
        wrong[offset..offset + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        let mut receiver = Receiver::default();
        assert!(receiver.decode(&wrong).is_err());
        assert!(receiver.known_templates().is_empty());
        assert!(matches!(
            receiver.decode(&packet).unwrap(),
            ResultProgram::Built(_)
        ));
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::{hint::black_box, time::Instant};
    use valle_draw::{Rect, program::*};

    #[test]
    #[ignore = "isolated kernel encode/decode benchmark; run optimized without other builds"]
    fn template_transport_benchmark() {
        for nodes in [200, 2000] {
            let mut builder = DrawProgramBuilder::new(Rect::new(0., 0., 640., 360.));
            let paint = builder.push_paint(Paint::Solid(LinearColor::new(1., 0., 0., 1.)));
            let mut boxes = BTreeMap::new();
            for i in 0..nodes {
                let x = (i % 200) as f64 * 3.;
                let y = (i / 200) as f64 * 3.;
                let path = builder.push_path(PathData {
                    verbs: vec![
                        PathVerb::MoveTo,
                        PathVerb::LineTo,
                        PathVerb::LineTo,
                        PathVerb::Close,
                    ],
                    points: vec![[x, y], [x + 2., y], [x + 2., y + 2.]],
                });
                let root = builder.push_node(Node::Path(PathNode {
                    path,
                    fill_rule: FillRule::NonZero,
                    fill: Some(paint),
                    stroke: None,
                }));
                builder.add_root(root);
                boxes.insert(format!("node-{i}"), [x as f32, y as f32, 2., 2.]);
            }
            let template = Arc::new(
                PaintTemplate::new(
                    builder.finish().unwrap(),
                    vec![PaintParameter { paint, slot: 0 }],
                    1,
                )
                .unwrap(),
            );
            let boxes = Arc::new(boxes);
            let mut timings = [Vec::new(), Vec::new()];
            let mut packet_bytes = [0, 0];
            for repeat in 0..5 {
                for version in if repeat % 2 == 0 { [0, 1] } else { [1, 0] } {
                    let mut encoder = Encoder::default();
                    let mut receiver = Receiver::default();
                    let mut run = |i: usize| {
                        let frame = template
                            .bind_frame(vec![LinearColor::new(i as f32 / 120., 0.2, 0.1, 1.)])
                            .unwrap();
                        let program = if version == 0 {
                            // Same materialized arena, original full-program + metadata packet.
                            CachedDrawProgram::new((**frame.program()).clone())
                        } else {
                            CachedDrawProgram::from_paint_frame(frame)
                        };
                        let packet = encoder
                            .encode(
                                BuiltMotionProgram {
                                    program: Arc::new(program),
                                    scene3d_frames: vec![],
                                    scene3d_sampling: vec![],
                                    layout_boxes: boxes.clone(),
                                },
                                &receiver.known_templates(),
                            )
                            .unwrap();
                        packet_bytes[version] = packet.len();
                        black_box(receiver.decode(&packet).unwrap());
                    };
                    run(0);
                    let started = Instant::now();
                    for frame in 0..120 {
                        run(frame);
                    }
                    timings[version].push(started.elapsed().as_secs_f64() * 1000. / 120.);
                }
            }
            for times in &mut timings {
                times.sort_by(f64::total_cmp);
            }
            eprintln!(
                "KERNEL_TRANSPORT_BENCH nodes={nodes} slots=1 old_bytes={} new_bytes={} old_ms={:.6} new_ms={:.6}",
                packet_bytes[0], packet_bytes[1], timings[0][2], timings[1][2]
            );
        }
    }
}
