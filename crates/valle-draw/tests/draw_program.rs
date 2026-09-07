use std::panic::{AssertUnwindSafe, catch_unwind};

use sha2::{Digest, Sha256};
use valle_draw::Rect;
use valle_draw::program::{
    BackdropRead, BackdropScope, BlendMode, Clip, DrawProgram, DrawProgramBuilder,
    DrawProgramError, Filter, Glyph, GlyphRun, Group, ImageNode, LinearColor, Mask, MaskMode, Node,
    NodeGeometry, NodeId, Paint, PathData, PathNode, PathStroke, PathVerb, RuntimeShaderNode,
    Scene3dNode, ShaderTextureBinding,
};
use valle_draw::requirements::{
    AlphaMode, ColorDomain, DestinationOperation, DigestBytes, ExternalTexture, FontKey, Insets,
    LocalBounds, RuntimeShaderKey, SamplingMode, Scene3dKey, TextureKind,
};

fn builder() -> DrawProgramBuilder {
    DrawProgramBuilder::new(Rect::new(0.0, 0.0, 64.0, 48.0))
}

fn fixture(reverse_arena_insertion: bool) -> DrawProgram {
    let mut builder = builder();

    let main_path_data = PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ],
        points: vec![[4.0, 4.0], [60.0, 4.0], [32.0, 44.0]],
    };
    let mask_path_data = PathData {
        verbs: vec![PathVerb::MoveTo, PathVerb::LineTo],
        points: vec![[0.0, 0.0], [64.0, 48.0]],
    };
    let (main_path, mask_path) = if reverse_arena_insertion {
        let mask = builder.push_path(mask_path_data);
        let main = builder.push_path(main_path_data);
        (main, mask)
    } else {
        let main = builder.push_path(main_path_data);
        let mask = builder.push_path(mask_path_data);
        (main, mask)
    };

    let white = Paint::Solid(LinearColor::new(0.8, 0.8, 0.8, 0.8));
    let mask_white = Paint::Solid(LinearColor::new(1.0, 1.0, 1.0, 1.0));
    let (main_paint, mask_paint) = if reverse_arena_insertion {
        let mask = builder.push_paint(mask_white);
        let main = builder.push_paint(white);
        (main, mask)
    } else {
        let main = builder.push_paint(white);
        let mask = builder.push_paint(mask_white);
        (main, mask)
    };

    let path = Node::Path(PathNode {
        path: main_path,
        fill_rule: Default::default(),
        fill: Some(main_paint),
        stroke: None,
    });
    let mask_path = Node::Path(PathNode {
        path: mask_path,
        fill_rule: Default::default(),
        fill: None,
        stroke: Some(PathStroke::new(mask_paint, 1.0)),
    });
    let image = Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:video-frame@1201/24000".into(),
            kind: TextureKind::Video,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: Some(50_042),
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(0.0, 0.0, 64.0, 48.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    });
    let glyphs = Node::GlyphRun(GlyphRun {
        font: FontKey {
            face_hash: DigestBytes::from_bytes([0x11; 32]),
            face_index: 0,
        },
        font_size: 16.0,
        glyphs: vec![
            Glyph {
                id: 7,
                x: 8.0,
                y: 28.0,
            },
            Glyph {
                id: 9,
                x: 24.0,
                y: 28.0,
            },
        ],
        bounds: Rect::new(8.0, 16.0, 28.0, 16.0),
        paint: main_paint,
        stroke: None,
        source_node: None,
        source_ranges: Vec::new(),
    });
    let shader = Node::RuntimeShader(RuntimeShaderNode {
        shader: RuntimeShaderKey {
            uri: "shader://glass/rim@1".into(),
            content_hash: DigestBytes::from_bytes([0x22; 32]),
            abi_hash: DigestBytes::from_bytes([0x33; 32]),
        },
        bounds: Rect::new(0.0, 0.0, 64.0, 48.0),
        uniforms: Vec::new(),
        textures: vec![ShaderTextureBinding {
            name: "noise".into(),
            texture: ExternalTexture {
                key: "asset:test-noise".into(),
                kind: TextureKind::Image,
                color_domain: ColorDomain::LinearRec2020,
                alpha: AlphaMode::Premultiplied,
                sample_time_micros: None,
            },
            sampling: SamplingMode::LinearClamp,
        }],
    });
    let scene = Node::Scene3d(Scene3dNode {
        scene: Scene3dKey {
            content_hash: valle_draw::requirements::DigestBytes::from_bytes([0x44; 32]),
            topology_hash: valle_draw::requirements::DigestBytes::from_bytes([0x55; 32]),
        },
        bounds: Rect::new(8.0, 8.0, 48.0, 32.0),
    });

    let mut semantic_nodes = vec![path, image, glyphs, shader, scene, mask_path];
    let ids = if reverse_arena_insertion {
        semantic_nodes.reverse();
        let inserted: Vec<_> = semantic_nodes
            .into_iter()
            .map(|node| builder.push_node(node))
            .collect();
        inserted.into_iter().rev().collect::<Vec<_>>()
    } else {
        semantic_nodes
            .into_iter()
            .map(|node| builder.push_node(node))
            .collect::<Vec<_>>()
    };
    let mask = *ids.last().expect("mask node");
    let root = builder.push_node(Node::Group(Group {
        children: ids[..5].to_vec(),
        glass: None,
        glass_foreground: None,
        transform: Default::default(),
        clip: Some(Clip::Rect(Rect::new(0.0, 0.0, 64.0, 48.0))),
        filters: vec![Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 3.0,
        }],
        mask: Some(Mask {
            source: mask,
            mode: MaskMode::Alpha,
        }),
        opacity: 0.75,
        internal_blend: BlendMode::Screen,
        isolated: true,
        backdrop: Some(BackdropRead {
            scope: BackdropScope::ScopeEntry("glass-card".into()),
            bounds: Rect::new(0.0, 0.0, 64.0, 48.0),
            footprint: Insets::uniform(12.0),
            sampling: valle_draw::requirements::SamplingMode::LinearClamp,
            filters: Vec::new(),
        }),
        shader: None,
        layer_bounds: None,
    }));
    builder.add_root(root);
    builder.finish().expect("valid fixture")
}

#[test]
fn structured_program_round_trips_and_derives_complete_requirements() {
    let program = fixture(false);
    program.validate().expect("valid program");
    assert_eq!(program.viewport(), Rect::new(0.0, 0.0, 64.0, 48.0));
    let requirements = program.requirements();
    assert_eq!(requirements.external_textures.len(), 2);
    assert_eq!(requirements.fonts.len(), 1);
    assert_eq!(requirements.runtime_shaders.len(), 1);
    assert_eq!(requirements.scene3d.len(), 1);
    assert_eq!(requirements.destination_uses.len(), 2);
    assert_eq!(
        requirements.destination_uses[0].scope,
        BackdropScope::ScopeEntry("glass-card".into())
    );
    assert_eq!(
        requirements.destination_uses[0].output_bounds,
        Rect::new(0.0, 0.0, 64.0, 48.0)
    );
    assert_eq!(
        requirements.destination_uses[0].sample_bounds,
        Rect::new(-12.0, -12.0, 88.0, 72.0)
    );
    assert!(matches!(
        requirements.destination_uses[0].operation,
        DestinationOperation::Backdrop {
            footprint,
            sampling: SamplingMode::LinearClamp,
        } if footprint == Insets::uniform(12.0)
    ));
    assert_eq!(
        requirements.destination_uses[1].scope,
        BackdropScope::Current
    );
    assert_eq!(
        requirements.destination_uses[1].output_bounds,
        Rect::new(-2.0, -2.0, 68.0, 52.0)
    );
    assert_eq!(
        requirements.destination_uses[1].sample_bounds,
        requirements.destination_uses[1].output_bounds
    );
    assert!(matches!(
        requirements.destination_uses[1].operation,
        DestinationOperation::Blend {
            mode: BlendMode::Screen,
        }
    ));
    assert_eq!(
        requirements.filter_footprint,
        Insets::new(6.0, 9.0, 6.0, 9.0)
    );
    assert_eq!(requirements.color_domains, [ColorDomain::LinearRec2020]);
    assert_eq!(requirements.alpha_modes, [AlphaMode::Premultiplied]);
    assert_eq!(
        requirements.content_bounds.rect(),
        Some(Rect::new(0.0, 0.0, 64.0, 48.0))
    );
    assert_eq!(
        requirements.output_bounds.rect(),
        Some(Rect::new(-2.0, -2.0, 68.0, 52.0))
    );
    assert_eq!(requirements.max_intermediate_pixels, 88 * 72);

    let packed = program.packed_bytes().expect("packed program");
    let decoded = DrawProgram::from_packed(&packed).expect("decode packed program");
    assert_eq!(decoded, program);
    assert_eq!(
        decoded.content_hash().unwrap(),
        program.content_hash().unwrap()
    );
}

#[test]
fn destination_requirements_stop_at_isolation_but_preserve_explicit_outer_scopes() {
    let mut builder = builder();
    let paint = builder.push_paint(Paint::Solid(LinearColor::new(0.5, 0.5, 0.5, 1.0)));
    let path = builder.push_path(PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ],
        points: vec![[4.0, 4.0], [20.0, 4.0], [4.0, 20.0]],
    });
    let leaf = builder.push_node(Node::Path(PathNode {
        path,
        fill_rule: Default::default(),
        fill: Some(paint),
        stroke: None,
    }));
    let mut local = Group::plain(vec![leaf]);
    local.internal_blend = BlendMode::Screen;
    local.backdrop = Some(BackdropRead {
        scope: BackdropScope::Current,
        bounds: Rect::new(4.0, 4.0, 16.0, 16.0),
        footprint: Insets::uniform(2.0),
        sampling: valle_draw::requirements::SamplingMode::LinearClamp,
        filters: Vec::new(),
    });
    let local = builder.push_node(Node::Group(local));

    let mut outer_scope = Group::plain(Vec::new());
    outer_scope.backdrop = Some(BackdropRead {
        scope: BackdropScope::LayerEntry("layer".into()),
        bounds: Rect::new(24.0, 4.0, 16.0, 16.0),
        footprint: Insets::uniform(1.0),
        sampling: valle_draw::requirements::SamplingMode::LinearClamp,
        filters: Vec::new(),
    });
    let outer_scope = builder.push_node(Node::Group(outer_scope));

    let mut isolated = Group::plain(vec![local, outer_scope]);
    isolated.isolated = true;
    let root = builder.push_node(Node::Group(isolated));
    builder.add_root(root);
    let program = builder.finish().unwrap();

    assert_eq!(program.requirements().destination_uses.len(), 1);
    assert_eq!(
        program.requirements().destination_uses[0].scope,
        BackdropScope::LayerEntry("layer".into())
    );
    assert!(matches!(
        program.requirements().destination_uses[0].operation,
        DestinationOperation::Backdrop { .. }
    ));
}

#[test]
fn canonical_nodes_retain_exact_validator_derived_geometry() {
    let rectangle = |left: f64, top: f64, right: f64, bottom: f64| PathData {
        verbs: vec![
            PathVerb::MoveTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::LineTo,
            PathVerb::Close,
        ],
        points: vec![[left, top], [right, top], [right, bottom], [left, bottom]],
    };
    let mut builder = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 12.0, 12.0));
    let child_path = builder.push_path(rectangle(2.0, 2.0, 6.0, 6.0));
    let mask_path = builder.push_path(rectangle(3.0, 3.0, 5.0, 5.0));
    let paint = builder.push_paint(Paint::Solid(LinearColor::new(1.0, 1.0, 1.0, 1.0)));
    let child = builder.push_node(Node::Path(PathNode {
        path: child_path,
        fill_rule: Default::default(),
        fill: Some(paint),
        stroke: None,
    }));
    let mask = builder.push_node(Node::Path(PathNode {
        path: mask_path,
        fill_rule: Default::default(),
        fill: Some(paint),
        stroke: None,
    }));
    let mut group = Group::plain(vec![child]);
    group.clip = Some(Clip::Rect(Rect::new(1.0, 1.0, 6.0, 6.0)));
    group.filters.push(Filter::Blur {
        sigma_x: 1.0,
        sigma_y: 1.0,
    });
    group.mask = Some(Mask {
        source: mask,
        mode: MaskMode::Alpha,
    });
    group.opacity = 0.5;
    let root = builder.push_node(Node::Group(group));
    builder.add_root(root);

    let program = builder.finish().expect("program geometry");
    assert_eq!(program.geometries().len(), program.nodes().len());
    let canonical_root = program.roots()[0];
    let Node::Group(group) = &program.nodes()[canonical_root.raw() as usize] else {
        panic!("canonical root must remain a group");
    };
    let canonical_child = group.children[0];
    let canonical_mask = group.mask.as_ref().expect("mask").source;
    assert_eq!(
        program.geometry(canonical_child),
        Some(NodeGeometry {
            content_bounds: LocalBounds::from_rect(Rect::new(2.0, 2.0, 4.0, 4.0)),
            output_bounds: LocalBounds::from_rect(Rect::new(2.0, 2.0, 4.0, 4.0)),
            max_intermediate_pixels: 0,
        })
    );
    assert_eq!(
        program.geometry(canonical_mask),
        Some(NodeGeometry {
            content_bounds: LocalBounds::from_rect(Rect::new(3.0, 3.0, 2.0, 2.0)),
            output_bounds: LocalBounds::from_rect(Rect::new(3.0, 3.0, 2.0, 2.0)),
            max_intermediate_pixels: 0,
        })
    );
    assert_eq!(
        program.geometry(canonical_root),
        Some(NodeGeometry {
            content_bounds: LocalBounds::from_rect(Rect::new(2.0, 2.0, 4.0, 4.0)),
            output_bounds: LocalBounds::from_rect(Rect::new(3.0, 3.0, 2.0, 2.0)),
            max_intermediate_pixels: 100,
        })
    );
    assert_eq!(program.requirements().max_intermediate_pixels, 100);

    let packed = program.packed_bytes().expect("packed program");
    let decoded = DrawProgram::from_packed(&packed).expect("decoded program");
    assert_eq!(decoded.geometries(), program.geometries());
    assert_eq!(decoded, program);
}

#[test]
fn viewport_and_intermediate_area_are_admission_contracts() {
    let zero = DrawProgramBuilder::new(Rect::new(0.0, 0.0, 0.0, 48.0));
    assert!(matches!(
        zero.finish(),
        Err(DrawProgramError::InvalidValue { location, .. }) if location == "viewport"
    ));

    let mut oversized = builder();
    let mut group = Group::plain(Vec::new());
    group.backdrop = Some(BackdropRead {
        scope: BackdropScope::Current,
        bounds: Rect::new(0.0, 0.0, 20_000.0, 20_000.0),
        footprint: Insets::default(),
        sampling: valle_draw::requirements::SamplingMode::LinearClamp,
        filters: Vec::new(),
    });
    let root = oversized.push_node(Node::Group(group));
    oversized.add_root(root);
    assert!(matches!(
        oversized.finish(),
        Err(DrawProgramError::BudgetExceeded { kind, actual, limit })
            if kind == "local intermediate pixels" && actual > limit
    ));
}

#[test]
fn glyph_run_requires_an_explicit_positive_local_font_size() {
    for font_size in [0.0, -1.0, f32::NAN] {
        let mut builder = builder();
        let paint = builder.push_paint(Paint::Solid(LinearColor::new(1.0, 1.0, 1.0, 1.0)));
        let glyph = builder.push_node(Node::GlyphRun(GlyphRun {
            font: FontKey {
                face_hash: DigestBytes::from_bytes([0x11; 32]),
                face_index: 0,
            },
            font_size,
            glyphs: vec![Glyph {
                id: 1,
                x: 8.0,
                y: 28.0,
            }],
            bounds: Rect::new(8.0, 16.0, 16.0, 16.0),
            paint,
            stroke: None,
            source_node: None,
            source_ranges: Vec::new(),
        }));
        builder.add_root(glyph);
        assert!(matches!(
            builder.finish(),
            Err(DrawProgramError::InvalidValue { location, .. })
                if location == "node[0].fontSize"
        ));
    }
}

#[test]
fn image_source_is_an_explicit_positive_normalized_content_rect() {
    for src in [
        Rect::new(0.0, 0.0, 0.0, 1.0),
        Rect::new(-0.1, 0.0, 0.5, 1.0),
        Rect::new(0.75, 0.0, 0.5, 1.0),
        Rect::new(f64::NAN, 0.0, 1.0, 1.0),
    ] {
        let mut builder = builder();
        let image = builder.push_node(Node::Image(ImageNode {
            texture: ExternalTexture {
                key: "asset://image".into(),
                kind: TextureKind::Image,
                color_domain: ColorDomain::LinearRec2020,
                alpha: AlphaMode::Premultiplied,
                sample_time_micros: None,
            },
            src,
            dst: Rect::new(0.0, 0.0, 64.0, 48.0),
            sampling: SamplingMode::LinearClamp,
            opacity: 1.0,
        }));
        builder.add_root(image);
        assert!(matches!(
            builder.finish(),
            Err(DrawProgramError::InvalidValue { location, .. })
                if location.starts_with("node[0].src")
        ));
    }
}

#[test]
fn canonical_bytes_ignore_arena_insertion_order_but_preserve_paint_order() {
    let forward = fixture(false);
    let reverse = fixture(true);
    assert_eq!(
        forward.packed_bytes().unwrap(),
        reverse.packed_bytes().unwrap()
    );
    assert_eq!(
        forward.content_hash().unwrap(),
        reverse.content_hash().unwrap()
    );
}

#[test]
fn invalid_ids_cycles_depth_and_undefined_reservations_fail_before_execution() {
    let mut invalid_id = builder();
    let root = invalid_id.push_node(Node::Group(Group::plain(vec![NodeId::from_raw(99)])));
    invalid_id.add_root(root);
    assert!(matches!(
        invalid_id.finish(),
        Err(DrawProgramError::InvalidNodeId { .. })
    ));

    let mut cycle = builder();
    let a = cycle.reserve_node();
    let b = cycle.reserve_node();
    cycle
        .define_node(a, Node::Group(Group::plain(vec![b])))
        .unwrap();
    cycle
        .define_node(b, Node::Group(Group::plain(vec![a])))
        .unwrap();
    cycle.add_root(a);
    assert!(matches!(
        cycle.finish(),
        Err(DrawProgramError::Cycle { .. })
    ));

    let mut undefined = builder();
    let root = undefined.reserve_node();
    undefined.add_root(root);
    assert!(matches!(
        undefined.finish(),
        Err(DrawProgramError::UndefinedNode { .. })
    ));

    let mut too_deep = builder();
    let empty_path = too_deep.push_path(PathData::empty());
    let mut child = too_deep.push_node(Node::Path(PathNode {
        path: empty_path,
        fill_rule: Default::default(),
        fill: None,
        stroke: None,
    }));
    for _ in 0..=valle_draw::program::MAX_GROUP_DEPTH {
        child = too_deep.push_node(Node::Group(Group::plain(vec![child])));
    }
    too_deep.add_root(child);
    assert!(matches!(
        too_deep.finish(),
        Err(DrawProgramError::DepthLimit { .. })
    ));
}

#[test]
fn packed_header_format_version_length_endianness_and_checksum_fail_closed() {
    let bytes = fixture(false).packed_bytes().unwrap();

    let mut format_version = bytes.clone();
    format_version[8..12]
        .copy_from_slice(&(valle_draw::program::DRAW_PROGRAM_FORMAT_VERSION + 1).to_le_bytes());
    format_version[28..60].fill(0);
    let checksum: [u8; 32] = Sha256::digest(&format_version).into();
    format_version[28..60].copy_from_slice(&checksum);
    assert!(matches!(
        DrawProgram::from_packed(&format_version),
        Err(valle_draw::program::PackedDrawError::UnsupportedFormatVersion { .. })
    ));

    let mut endian = bytes.clone();
    endian[12..16].copy_from_slice(&0x0403_0201_u32.to_le_bytes());
    assert!(matches!(
        DrawProgram::from_packed(&endian),
        Err(valle_draw::program::PackedDrawError::WrongEndianness { .. })
    ));

    let mut length = bytes.clone();
    length[20..28].copy_from_slice(&0_u64.to_le_bytes());
    assert!(matches!(
        DrawProgram::from_packed(&length),
        Err(valle_draw::program::PackedDrawError::LengthMismatch { .. })
    ));

    let mut checksum = bytes;
    *checksum.last_mut().unwrap() ^= 0x80;
    assert!(matches!(
        DrawProgram::from_packed(&checksum),
        Err(valle_draw::program::PackedDrawError::ChecksumMismatch)
    ));
}

#[test]
fn arbitrary_packed_input_never_panics() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let cases = std::env::var("VALLE_NIGHTLY_FUZZ_CASES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(512)
        .min(100_000);
    for case in 0..cases {
        let len = if case < 512 {
            case
        } else {
            (state as usize) % 4096
        };
        let mut bytes = vec![0_u8; len];
        for byte in &mut bytes {
            state ^= state << 7;
            state ^= state >> 9;
            state ^= state << 8;
            *byte = state as u8;
        }
        assert!(catch_unwind(AssertUnwindSafe(|| DrawProgram::from_packed(&bytes))).is_ok());
    }
}

#[test]
fn forest_and_side_tables_have_single_owners() {
    let mut shared = builder();
    let leaf = shared.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:test-shared".into(),
            kind: TextureKind::Image,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: None,
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(0.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    let left = shared.push_node(Node::Group(Group::plain(vec![leaf])));
    let right = shared.push_node(Node::Group(Group::plain(vec![leaf])));
    shared.add_root(left);
    shared.add_root(right);
    assert!(matches!(
        shared.finish(),
        Err(DrawProgramError::MultipleParents { id }) if id == leaf
    ));

    let mut orphan = builder();
    let orphan_path = orphan.push_path(PathData::empty());
    let root = orphan.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:test-root".into(),
            kind: TextureKind::Image,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: None,
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(0.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    orphan.add_root(root);
    assert!(matches!(
        orphan.finish(),
        Err(DrawProgramError::UnreachablePath { id }) if id == orphan_path
    ));
}

#[test]
fn requirements_reject_conflicting_external_interpretations() {
    let mut builder = builder();
    let first = builder.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:test-same-content".into(),
            kind: TextureKind::Image,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: None,
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(0.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    let second = builder.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:test-same-content".into(),
            kind: TextureKind::Video,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: Some(0),
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(1.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    let root = builder.push_node(Node::Group(Group::plain(vec![first, second])));
    builder.add_root(root);
    assert!(matches!(
        builder.finish(),
        Err(DrawProgramError::ConflictingRequirement { .. })
    ));
}

#[test]
fn video_requirements_keep_producer_local_samples_for_one_asset_distinct() {
    let mut builder = builder();
    let first = builder.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset://shared-video".into(),
            kind: TextureKind::Video,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: Some(250_000),
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(0.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    let second = builder.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset://shared-video".into(),
            kind: TextureKind::Video,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: Some(750_000),
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(1.0, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    let root = builder.push_node(Node::Group(Group::plain(vec![first, second])));
    builder.add_root(root);
    let program = builder
        .finish()
        .expect("distinct video samples are distinct slots");
    assert_eq!(
        program
            .requirements()
            .external_textures
            .iter()
            .map(|texture| texture.sample_time_micros)
            .collect::<Vec<_>>(),
        vec![Some(250_000), Some(750_000)]
    );
    let decoded = DrawProgram::from_packed(&program.packed_bytes().unwrap()).unwrap();
    assert_eq!(decoded.requirements(), program.requirements());
}

#[test]
fn video_texture_timestamp_domain_matches_the_cross_runtime_safe_integer_contract() {
    for sample_time_micros in [None, Some(-1), Some(9_007_199_254_740_992)] {
        let mut builder = builder();
        let image = builder.push_node(Node::Image(ImageNode {
            texture: ExternalTexture {
                key: "asset://video".into(),
                kind: TextureKind::Video,
                color_domain: ColorDomain::LinearRec2020,
                alpha: AlphaMode::Premultiplied,
                sample_time_micros,
            },
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            sampling: SamplingMode::LinearClamp,
            opacity: 1.0,
        }));
        builder.add_root(image);
        assert!(
            builder.finish().is_err(),
            "invalid sample {sample_time_micros:?}"
        );
    }

    for kind in [TextureKind::Image, TextureKind::Generated] {
        let mut builder = builder();
        let image = builder.push_node(Node::Image(ImageNode {
            texture: ExternalTexture {
                key: "asset://static".into(),
                kind,
                color_domain: ColorDomain::LinearRec2020,
                alpha: AlphaMode::Premultiplied,
                sample_time_micros: Some(0),
            },
            src: Rect::new(0.0, 0.0, 1.0, 1.0),
            dst: Rect::new(0.0, 0.0, 1.0, 1.0),
            sampling: SamplingMode::LinearClamp,
            opacity: 1.0,
        }));
        builder.add_root(image);
        assert!(
            builder.finish().is_err(),
            "non-video texture kind {kind:?} must not carry a timestamp"
        );
    }
}

#[test]
fn non_finite_or_non_premultiplied_values_never_reach_execution() {
    let mut non_finite = builder();
    let root = non_finite.push_node(Node::Image(ImageNode {
        texture: ExternalTexture {
            key: "asset:test-nan".into(),
            kind: TextureKind::Image,
            color_domain: ColorDomain::LinearRec2020,
            alpha: AlphaMode::Premultiplied,
            sample_time_micros: None,
        },
        src: Rect::new(0.0, 0.0, 1.0, 1.0),
        dst: Rect::new(f64::NAN, 0.0, 1.0, 1.0),
        sampling: SamplingMode::LinearClamp,
        opacity: 1.0,
    }));
    non_finite.add_root(root);
    assert!(matches!(
        non_finite.finish(),
        Err(DrawProgramError::InvalidValue { .. })
    ));

    let mut transparent_rgb = builder();
    let path = transparent_rgb.push_path(PathData::empty());
    let paint = transparent_rgb.push_paint(Paint::Solid(LinearColor::new(1.0, 0.0, 0.0, 0.0)));
    let root = transparent_rgb.push_node(Node::Path(PathNode {
        path,
        fill_rule: Default::default(),
        fill: Some(paint),
        stroke: None,
    }));
    transparent_rgb.add_root(root);
    assert!(matches!(
        transparent_rgb.finish(),
        Err(DrawProgramError::InvalidValue { .. })
    ));
}

#[test]
fn nested_filter_footprints_accumulate_conservatively() {
    let mut builder = builder();
    let mut inner_group = Group::plain(Vec::new());
    inner_group.filters.push(Filter::Blur {
        sigma_x: 2.0,
        sigma_y: 1.0,
    });
    let inner = builder.push_node(Node::Group(inner_group));
    let mut outer_group = Group::plain(vec![inner]);
    outer_group.filters.push(Filter::Blur {
        sigma_x: 3.0,
        sigma_y: 4.0,
    });
    let outer = builder.push_node(Node::Group(outer_group));
    builder.add_root(outer);
    let program = builder.finish().unwrap();
    assert_eq!(
        program.requirements().filter_footprint,
        Insets::new(15.0, 15.0, 15.0, 15.0)
    );
}

#[test]
fn path_grammar_is_closed_instead_of_backend_defined() {
    let mut builder = builder();
    let path = builder.push_path(PathData {
        verbs: vec![PathVerb::LineTo],
        points: vec![[1.0, 1.0]],
    });
    let root = builder.push_node(Node::Path(PathNode {
        path,
        fill_rule: Default::default(),
        fill: None,
        stroke: None,
    }));
    builder.add_root(root);
    assert!(matches!(
        builder.finish(),
        Err(DrawProgramError::InvalidValue { .. })
    ));
}
