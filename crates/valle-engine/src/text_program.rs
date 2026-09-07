//! Compiled caption shaping into the structured drawing narrow waist.

use cosmic_text::{Attrs, Buffer, Color as CosmicColor, Family, Metrics, Shaping, Weight, fontdb};
use thiserror::Error;
use valle_draw::program::{
    Affine2d, Clip, DrawProgram, DrawProgramBuilder, Filter, Glyph, GlyphRun, Group, LinearColor,
    MAX_FILTER_SIGMA, Node, NodeId, Paint, Transform2d,
};
use valle_draw::{Rect, Rgba};

use super::{CaptionBehavior, CaptionDrawInput, DEFAULT_LINE_SPACING, TextSemantics};

#[derive(Debug, Error)]
pub enum TextProgramError {
    #[error("caption viewport must be finite and positive")]
    InvalidViewport,
    #[error("product caption shaping requires at least one admitted semantic font")]
    NoFonts,
    #[error("shaping selected fontdb face {face:?} without an admitted semantic identity")]
    MissingFontIdentity { face: fontdb::ID },
    #[error("caption glyph source range exceeds the DrawProgram u32 address space")]
    SourceRangeOverflow,
    #[error("karaoke caption run {run_index} has no trustworthy glyph source ranges")]
    MissingKaraokeSourceRanges { run_index: usize },
    #[error("caption DrawProgram is invalid: {0}")]
    InvalidProgram(#[from] valle_draw::program::DrawProgramError),
}

#[derive(Clone)]
struct GlyphBucket {
    run_index: usize,
    font_id: fontdb::ID,
    font: valle_draw::requirements::FontKey,
    font_size: f32,
    glyphs: Vec<Glyph>,
    source_ranges: Vec<[u32; 2]>,
    source_node: String,
    source_valid: bool,
    bounds: Option<Rect>,
}

impl GlyphBucket {
    fn rect(&self) -> Rect {
        self.bounds.unwrap_or(Rect::new(0.0, 0.0, 0.0, 0.0))
    }
}

impl TextSemantics {
    /// Shape one evaluated caption from the pinned compiled render.
    pub(crate) fn build_caption_draw_program(
        &mut self,
        input: &CaptionDrawInput<'_>,
        width: u32,
        height: u32,
    ) -> Result<DrawProgram, TextProgramError> {
        if width == 0
            || height == 0
            || input.run_styles.len() != input.runs.len()
            || input.run_styles.iter().any(|style| {
                !style.font_size.is_finite()
                    || style.font_size <= 0.0
                    || style.font_size > valle_timeline::internal::document::MAX_CAPTION_FONT_SIZE
                    || !(1..=1000).contains(&style.font_weight)
            })
            || input.shadow.is_some_and(|shadow| {
                shadow.offset.iter().any(|value| !value.is_finite())
                    || !shadow.blur_sigma.is_finite()
                    || shadow.blur_sigma < 0.0
                    || shadow.blur_sigma > f64::from(MAX_FILTER_SIGMA)
            })
            || !input.presentation.opacity.is_finite()
            || !(0.0..=1.0).contains(&input.presentation.opacity)
            || input
                .presentation
                .translation
                .iter()
                .any(|value| !value.is_finite())
            || !input.presentation.scale.is_finite()
            || input.presentation.scale < 0.0
            || !input.presentation.rotation.is_finite()
            || !input
                .presentation
                .clip_inset
                .iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            || !input.presentation.blur_sigma.is_finite()
            || input.presentation.blur_sigma < 0.0
            || input.presentation.blur_sigma > f64::from(MAX_FILTER_SIGMA)
            || !input.region.iter().all(|value| value.is_finite())
            || input.region[2] <= 0.0
            || input.region[3] <= 0.0
            || input.region[0] < 0.0
            || input.region[1] < 0.0
            || input.region[0] + input.region[2] > 1.0
            || input.region[1] + input.region[3] > 1.0
            || !input.align.iter().all(|value| (0.0..=1.0).contains(value))
            || matches!(
                input.behavior,
                CaptionBehavior::Scroll {
                    speed,
                    elapsed_seconds,
                    ..
                } if !speed.is_finite() || !elapsed_seconds.is_finite()
            )
        {
            return Err(TextProgramError::InvalidViewport);
        }
        if self.program_fonts.is_empty() {
            return Err(TextProgramError::NoFonts);
        }

        let viewport = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        let [top, right, bottom, left] = input.presentation.clip_inset;
        if input.presentation.opacity <= 0.0
            || input.presentation.scale <= 0.0
            || left + right >= 1.0
            || top + bottom >= 1.0
        {
            return DrawProgramBuilder::new(viewport)
                .finish()
                .map_err(Into::into);
        }
        let region = Rect::new(
            input.region[0] * f64::from(width),
            input.region[1] * f64::from(height),
            input.region[2] * f64::from(width),
            input.region[3] * f64::from(height),
        );
        let font_size = input
            .run_styles
            .iter()
            .map(|style| style.font_size)
            .fold(1.0_f64, f64::max) as f32;
        let line_metrics = Metrics::new(font_size, font_size * DEFAULT_LINE_SPACING as f32);
        let mut buffer = Buffer::new(&mut self.fs, line_metrics);
        buffer.set_wrap(cosmic_text::Wrap::WordOrGlyph);
        buffer.set_size(Some(region.width as f32), Some(region.height as f32));
        let spans =
            input
                .runs
                .iter()
                .zip(input.run_styles)
                .enumerate()
                .map(|(index, (text, style))| {
                    let metrics = Metrics::new(
                        style.font_size as f32,
                        style.font_size as f32 * DEFAULT_LINE_SPACING as f32,
                    );
                    (
                        text.as_str(),
                        Attrs::new()
                            .family(Family::Name(input.font_family))
                            .color(CosmicColor::rgba(
                                style.color[0],
                                style.color[1],
                                style.color[2],
                                style.color[3],
                            ))
                            .weight(Weight(style.font_weight))
                            .metadata(index + 1)
                            .metrics(metrics),
                    )
                });
        let horizontal_scroll = matches!(
            input.behavior,
            CaptionBehavior::Scroll {
                horizontal: true,
                ..
            }
        );
        let paragraph_align = Some(if horizontal_scroll || input.align[0] <= 0.25 {
            cosmic_text::Align::Left
        } else if input.align[0] >= 0.75 {
            cosmic_text::Align::Right
        } else {
            cosmic_text::Align::Center
        });
        buffer.set_rich_text(spans, &Attrs::new(), Shaping::Advanced, paragraph_align);
        buffer.shape_until_scroll(&mut self.fs, false);

        let mut block_width = 0.0_f32;
        let mut block_height = 0.0_f32;
        for line in buffer.layout_runs() {
            block_width = block_width.max(line.line_w);
            block_height = block_height.max(line.line_top + line.line_height);
        }
        let mut builder = DrawProgramBuilder::new(viewport);
        if block_width <= 0.0 || block_height <= 0.0 {
            return builder.finish().map_err(Into::into);
        }

        let mut offset_x = region.x;
        let mut offset_y =
            region.y + (region.height - f64::from(block_height)).max(0.0) * input.align[1];
        let clip = matches!(input.behavior, CaptionBehavior::Scroll { .. });
        if let CaptionBehavior::Scroll {
            horizontal,
            speed,
            elapsed_seconds,
        } = input.behavior
        {
            let distance = if horizontal {
                region.width + f64::from(block_width)
            } else {
                region.height + f64::from(block_height)
            };
            let phase =
                (speed.abs() * elapsed_seconds.max(0.0)).rem_euclid(distance.max(f64::EPSILON));
            if horizontal {
                offset_x = if speed >= 0.0 {
                    region.x - f64::from(block_width) + phase
                } else {
                    region.x + region.width - phase
                };
            } else {
                offset_y = if speed >= 0.0 {
                    region.y + region.height - phase
                } else {
                    region.y - f64::from(block_height) + phase
                };
            }
        }

        let (buckets, mut block_bounds) =
            collect_glyph_buckets(&buffer, input.runs, &self.program_fonts, offset_x, offset_y)?;
        if block_bounds.width > 0.0 {
            block_bounds = Rect::from_edges(
                block_bounds.left(),
                offset_y,
                block_bounds.right(),
                offset_y + f64::from(block_height),
            );
        }
        let mut roots = Vec::new();
        match input.behavior {
            CaptionBehavior::Karaoke { reveal_bytes } => {
                let mut author_starts = Vec::with_capacity(input.runs.len());
                let mut author_start = 0_usize;
                for run in input.runs {
                    author_starts.push(author_start);
                    author_start = author_start.saturating_add(run.len());
                }
                let mut revealed_roots = Vec::new();
                for bucket in &buckets {
                    if bucket.source_ranges.len() != bucket.glyphs.len() {
                        return Err(TextProgramError::MissingKaraokeSourceRanges {
                            run_index: bucket.run_index,
                        });
                    }
                    let mut dim_glyphs = Vec::new();
                    let mut dim_ranges = Vec::new();
                    let mut revealed_glyphs = Vec::new();
                    let mut revealed_ranges = Vec::new();
                    let author_start = author_starts[bucket.run_index];
                    for (glyph, range) in bucket.glyphs.iter().zip(&bucket.source_ranges) {
                        let revealed =
                            author_start.saturating_add(range[1] as usize) <= reveal_bytes;
                        if revealed {
                            revealed_glyphs.push(*glyph);
                            revealed_ranges.push(*range);
                        } else {
                            dim_glyphs.push(*glyph);
                            dim_ranges.push(*range);
                        }
                    }
                    if !dim_glyphs.is_empty() {
                        let mut color = input.run_styles[bucket.run_index].color;
                        color[3] = ((u16::from(color[3]) * 3) / 8) as u8;
                        let paint = builder.push_paint(Paint::Solid(linear_color(color)));
                        roots.push(push_bucket(
                            &mut builder,
                            bucket,
                            paint,
                            dim_glyphs,
                            dim_ranges,
                        ));
                    }
                    if !revealed_glyphs.is_empty() {
                        let paint = builder.push_paint(Paint::Solid(linear_color(
                            input.run_styles[bucket.run_index].color,
                        )));
                        revealed_roots.push(push_bucket(
                            &mut builder,
                            bucket,
                            paint,
                            revealed_glyphs,
                            revealed_ranges,
                        ));
                    }
                }
                roots.extend(revealed_roots);
            }
            CaptionBehavior::Static | CaptionBehavior::Scroll { .. } => {
                for bucket in &buckets {
                    let paint = builder.push_paint(Paint::Solid(linear_color(
                        input.run_styles[bucket.run_index].color,
                    )));
                    roots.push(push_bucket(
                        &mut builder,
                        bucket,
                        paint,
                        bucket.glyphs.clone(),
                        bucket.source_ranges.clone(),
                    ));
                }
            }
        }

        if let Some(mut root) = group_children(&mut builder, roots) {
            if let Some(shadow) = input.shadow {
                let mut group = Group::plain(vec![root]);
                group.filters.push(Filter::DropShadow {
                    offset: [shadow.offset[0] as f32, shadow.offset[1] as f32],
                    sigma_x: shadow.blur_sigma as f32,
                    sigma_y: shadow.blur_sigma as f32,
                    color: linear_color(shadow.color),
                });
                root = builder.push_node(Node::Group(group));
            }

            let presentation = input.presentation;
            let clipped_width = block_bounds.width * (1.0 - left - right);
            let clipped_height = block_bounds.height * (1.0 - top - bottom);
            debug_assert!(clipped_width > 0.0 && clipped_height > 0.0);
            let center_x = block_bounds.x + block_bounds.width * 0.5;
            let center_y = block_bounds.y + block_bounds.height * 0.5;
            let transform = Affine2d::translate(-center_x, -center_y)
                .then(Affine2d::scale(presentation.scale, presentation.scale))
                // Timeline angles are unwrapped radians; DrawProgram's affine
                // constructor deliberately accepts degrees.
                .then(Affine2d::rotate(presentation.rotation.to_degrees()))
                .then(Affine2d::translate(
                    center_x + presentation.translation[0],
                    center_y + presentation.translation[1],
                ));
            let mut group = Group::plain(vec![root]);
            group.opacity = presentation.opacity as f32;
            group.transform = Transform2d::from_affine(transform);
            if presentation.clip_inset.iter().any(|value| *value > 0.0) {
                group.clip = Some(Clip::Rect(Rect::new(
                    block_bounds.x + block_bounds.width * left,
                    block_bounds.y + block_bounds.height * top,
                    clipped_width,
                    clipped_height,
                )));
            }
            if presentation.blur_sigma > 0.0 {
                group.filters.push(Filter::Blur {
                    sigma_x: presentation.blur_sigma as f32,
                    sigma_y: presentation.blur_sigma as f32,
                });
            }
            root = builder.push_node(Node::Group(group));
            if clip {
                let mut group = Group::plain(vec![root]);
                group.clip = Some(Clip::Rect(region));
                root = builder.push_node(Node::Group(group));
            }
            builder.add_root(root);
        }
        builder.finish().map_err(Into::into)
    }
}

fn collect_glyph_buckets(
    buffer: &Buffer,
    author_runs: &[String],
    identities: &std::collections::HashMap<fontdb::ID, valle_draw::requirements::FontKey>,
    offset_x: f64,
    offset_y: f64,
) -> Result<(Vec<GlyphBucket>, Rect), TextProgramError> {
    let mut author_starts = Vec::with_capacity(author_runs.len());
    let mut combined = String::new();
    for run in author_runs {
        author_starts.push(combined.len());
        combined.push_str(run);
    }
    let mut line_starts = vec![0_usize];
    for (index, byte) in combined.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(index + 1);
        }
    }

    let mut buckets = Vec::<GlyphBucket>::new();
    let mut block_bounds = None;
    for line in buffer.layout_runs() {
        let line_start = line_starts.get(line.line_i).copied().unwrap_or(0);
        for glyph in line.glyphs {
            let run_index = glyph.metadata.saturating_sub(1);
            if glyph.metadata == 0 || run_index >= author_runs.len() {
                continue;
            }
            let font = identities.get(&glyph.font_id).cloned().ok_or(
                TextProgramError::MissingFontIdentity {
                    face: glyph.font_id,
                },
            )?;
            let bucket_index = buckets
                .iter()
                .position(|bucket| {
                    bucket.run_index == run_index
                        && bucket.font_id == glyph.font_id
                        && bucket.font_size.to_bits() == glyph.font_size.to_bits()
                })
                .unwrap_or_else(|| {
                    buckets.push(GlyphBucket {
                        run_index,
                        font_id: glyph.font_id,
                        font,
                        font_size: glyph.font_size,
                        glyphs: Vec::new(),
                        source_ranges: Vec::new(),
                        source_node: format!("caption/run/{run_index}"),
                        source_valid: true,
                        bounds: None,
                    });
                    buckets.len() - 1
                });
            let x = offset_x + f64::from(glyph.x + glyph.font_size * glyph.x_offset);
            let y = offset_y + f64::from(line.line_y + glyph.y - glyph.font_size * glyph.y_offset);
            let layout_cell = Rect::from_edges(
                x,
                offset_y + f64::from(line.line_top),
                x + f64::from(glyph.w),
                offset_y + f64::from(line.line_top + line.line_height),
            );
            block_bounds =
                Some(block_bounds.map_or(layout_cell, |current| union(current, layout_cell)));
            let bucket = &mut buckets[bucket_index];
            bucket.glyphs.push(Glyph {
                id: u32::from(glyph.glyph_id),
                x,
                y,
            });
            // Conservative shaper-owned bounds. Exact outlines remain an executor concern.
            let em = f64::from(glyph.font_size);
            let cell = Rect::from_edges(
                x - em * 2.0,
                y - em * 2.0,
                x + f64::from(glyph.w) + em * 2.0,
                y + em * 2.0,
            );
            bucket.bounds = Some(bucket.bounds.map_or(cell, |current| union(current, cell)));

            let global_start = line_start.saturating_add(glyph.start);
            let global_end = line_start.saturating_add(glyph.end);
            let owner_start = author_starts[run_index];
            let owner_end = owner_start.saturating_add(author_runs[run_index].len());
            if global_start < owner_start || global_end > owner_end || global_start > global_end {
                bucket.source_valid = false;
            } else {
                bucket.source_ranges.push([
                    u32::try_from(global_start - owner_start)
                        .map_err(|_| TextProgramError::SourceRangeOverflow)?,
                    u32::try_from(global_end - owner_start)
                        .map_err(|_| TextProgramError::SourceRangeOverflow)?,
                ]);
            }
        }
    }
    for bucket in &mut buckets {
        if !bucket.source_valid || bucket.source_ranges.len() != bucket.glyphs.len() {
            bucket.source_ranges.clear();
        }
    }
    Ok((
        buckets,
        block_bounds.unwrap_or(Rect::new(offset_x, offset_y, 0.0, 0.0)),
    ))
}

fn push_bucket(
    builder: &mut DrawProgramBuilder,
    bucket: &GlyphBucket,
    paint: valle_draw::program::PaintId,
    glyphs: Vec<Glyph>,
    source_ranges: Vec<[u32; 2]>,
) -> NodeId {
    let source_node = (!source_ranges.is_empty()).then(|| bucket.source_node.clone());
    builder.push_node(Node::GlyphRun(GlyphRun {
        font: bucket.font.clone(),
        font_size: bucket.font_size,
        glyphs,
        bounds: bucket.rect(),
        paint,
        stroke: None,
        source_node,
        source_ranges,
    }))
}

fn group_children(builder: &mut DrawProgramBuilder, children: Vec<NodeId>) -> Option<NodeId> {
    match children.as_slice() {
        [] => None,
        [only] => Some(*only),
        _ => Some(builder.push_node(Node::Group(Group::plain(children)))),
    }
}

fn linear_color(color: [u8; 4]) -> LinearColor {
    LinearColor::from_srgb8(Rgba::new(color[0], color[1], color[2], color[3]))
}

fn union(left: Rect, right: Rect) -> Rect {
    Rect::from_edges(
        left.left().min(right.left()),
        left.top().min(right.top()),
        left.right().max(right.right()),
        left.bottom().max(right.bottom()),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const DEFAULT_RUN_STYLES: [super::super::CaptionRunStyle; 1] =
        [super::super::CaptionRunStyle {
            font_size: 32.0,
            color: [255, 255, 255, 255],
            font_weight: 400,
        }];

    fn input<'a>(runs: &'a [String]) -> CaptionDrawInput<'a> {
        assert_eq!(runs.len(), DEFAULT_RUN_STYLES.len());
        CaptionDrawInput {
            runs,
            run_styles: &DEFAULT_RUN_STYLES,
            font_family: crate::resource::DEFAULT_FONT_FAMILY,
            shadow: None,
            region: [0.0, 0.0, 1.0, 1.0],
            align: [0.5, 0.5],
            presentation: super::super::CaptionPresentation {
                opacity: 1.0,
                translation: [0.0, 0.0],
                scale: 1.0,
                rotation: 0.0,
                clip_inset: [0.0; 4],
                blur_sigma: 0.0,
            },
            behavior: CaptionBehavior::Static,
        }
    }

    #[test]
    fn compiled_shaper_rejects_unadmitted_fonts() {
        let mut semantics = TextSemantics::product();
        let runs = vec!["hello".to_owned()];
        assert!(matches!(
            semantics.build_caption_draw_program(&input(&runs), 320, 180),
            Err(TextProgramError::NoFonts)
        ));
    }

    #[test]
    fn compiled_shaper_emits_keyed_glyphs_and_author_ranges() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["hello".to_owned()];
        let program = semantics
            .build_caption_draw_program(&input(&runs), 320, 180)
            .unwrap();
        assert!(program.nodes().iter().any(|node| {
            matches!(
                node,
                Node::GlyphRun(run)
                    if run.font.face_hash.as_hex() == digest
                        && run.source_node.as_deref() == Some("caption/run/0")
                        && run.source_ranges.len() == run.glyphs.len()
            )
        }));
    }

    #[test]
    fn compiled_shaper_rejects_non_finite_scroll_state() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["hello".to_owned()];
        let mut caption = input(&runs);
        caption.behavior = CaptionBehavior::Scroll {
            horizontal: true,
            speed: f64::NAN,
            elapsed_seconds: 0.0,
        };
        assert!(matches!(
            semantics.build_caption_draw_program(&caption, 320, 180),
            Err(TextProgramError::InvalidViewport)
        ));
    }

    #[test]
    fn compiled_shaper_rejects_sigma_above_draw_contract_before_shaping() {
        let mut semantics = TextSemantics::product();
        let runs = vec!["hello".to_owned()];

        let mut shadow = input(&runs);
        shadow.shadow = Some(super::super::CaptionShadow {
            color: [0, 0, 0, 255],
            offset: [0.0, 0.0],
            blur_sigma: f64::from(MAX_FILTER_SIGMA) + 1.0,
        });
        assert!(matches!(
            semantics.build_caption_draw_program(&shadow, 320, 180),
            Err(TextProgramError::InvalidViewport)
        ));

        let mut presentation = input(&runs);
        presentation.presentation.blur_sigma = f64::from(MAX_FILTER_SIGMA) + 1.0;
        assert!(matches!(
            semantics.build_caption_draw_program(&presentation, 320, 180),
            Err(TextProgramError::InvalidViewport)
        ));
    }

    #[test]
    fn horizontal_scroll_uses_block_local_alignment_and_wraps_at_its_period() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["scrolling".to_owned()];
        let mut caption = input(&runs);
        caption.region = [0.25, 0.25, 0.5, 0.5];
        caption.align = [1.0, 0.5];
        caption.behavior = CaptionBehavior::Scroll {
            horizontal: true,
            speed: 180.0,
            elapsed_seconds: 0.0,
        };

        let at_start = semantics
            .build_caption_draw_program(&caption, 320, 180)
            .unwrap();
        let glyph_x = at_start
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::GlyphRun(run) => Some(run.glyphs.iter().map(|glyph| glyph.x)),
                _ => None,
            })
            .flatten()
            .collect::<Vec<_>>();
        let minimum_x = glyph_x.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum_x = glyph_x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let region_left = 80.0;
        assert!(maximum_x < region_left);
        assert!(at_start.nodes().iter().any(|node| matches!(
            node,
            Node::Group(group)
                if group.clip == Some(Clip::Rect(Rect::new(80.0, 45.0, 160.0, 90.0)))
        )));

        let block_width = region_left - minimum_x;
        let period_seconds = (160.0 + block_width) / 180.0;
        caption.behavior = CaptionBehavior::Scroll {
            horizontal: true,
            speed: 180.0,
            elapsed_seconds: period_seconds,
        };
        let at_period = semantics
            .build_caption_draw_program(&caption, 320, 180)
            .unwrap();
        assert_eq!(
            at_start.packed_bytes().unwrap(),
            at_period.packed_bytes().unwrap()
        );
    }

    #[test]
    fn scroll_speed_is_signed_canvas_pixels_per_second_on_each_axis() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["scrolling".to_owned()];

        let glyph_position = |program: &DrawProgram| {
            program
                .nodes()
                .iter()
                .find_map(|node| match node {
                    Node::GlyphRun(run) => run.glyphs.first().map(|glyph| [glyph.x, glyph.y]),
                    _ => None,
                })
                .expect("scrolling caption emits at least one glyph")
        };

        for (horizontal, speed, expected_delta) in [
            (true, 80.0, [20.0, 0.0]),
            (true, -80.0, [-20.0, 0.0]),
            (false, 80.0, [0.0, -20.0]),
            (false, -80.0, [0.0, 20.0]),
        ] {
            let mut caption = input(&runs);
            caption.behavior = CaptionBehavior::Scroll {
                horizontal,
                speed,
                elapsed_seconds: 0.0,
            };
            let start = glyph_position(
                &semantics
                    .build_caption_draw_program(&caption, 320, 180)
                    .unwrap(),
            );
            caption.behavior = CaptionBehavior::Scroll {
                horizontal,
                speed,
                elapsed_seconds: 0.25,
            };
            let later = glyph_position(
                &semantics
                    .build_caption_draw_program(&caption, 320, 180)
                    .unwrap(),
            );
            assert!((later[0] - start[0] - expected_delta[0]).abs() < 1.0e-6);
            assert!((later[1] - start[1] - expected_delta[1]).abs() < 1.0e-6);
        }
    }

    #[test]
    fn compiled_shaper_emits_a_valid_empty_program_when_presentation_is_hidden() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["hidden".to_owned()];
        let mut caption = input(&runs);
        caption.presentation.opacity = 0.0;

        let program = semantics
            .build_caption_draw_program(&caption, 320, 180)
            .unwrap();

        assert!(program.nodes().is_empty());
        assert!(program.roots().is_empty());
    }

    #[test]
    fn compiled_shaper_karaoke_does_not_double_composite_revealed_alpha() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["A\n".to_owned(), "B".to_owned()];
        let run_styles = [
            super::super::CaptionRunStyle {
                font_size: 32.0,
                color: [255, 0, 0, 128],
                font_weight: 400,
            },
            super::super::CaptionRunStyle {
                font_size: 32.0,
                color: [0, 255, 0, 64],
                font_weight: 400,
            },
        ];
        let caption = CaptionDrawInput {
            runs: &runs,
            run_styles: &run_styles,
            font_family: crate::resource::DEFAULT_FONT_FAMILY,
            shadow: None,
            region: [0.0, 0.0, 1.0, 1.0],
            align: [0.5, 0.5],
            presentation: super::super::CaptionPresentation {
                opacity: 1.0,
                translation: [0.0, 0.0],
                scale: 1.0,
                rotation: 0.0,
                clip_inset: [0.0; 4],
                blur_sigma: 0.0,
            },
            behavior: CaptionBehavior::Karaoke { reveal_bytes: 2 },
        };

        let program = semantics
            .build_caption_draw_program(&caption, 320, 180)
            .unwrap();
        let run_alphas = program
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::GlyphRun(run) => {
                    let Paint::Solid(color) = program.paints()[run.paint.raw() as usize] else {
                        panic!("caption glyph paint must be solid");
                    };
                    Some((run.source_node.as_deref(), color.alpha))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(run_alphas.iter().any(|(owner, alpha)| {
            *owner == Some("caption/run/0") && (*alpha - 128.0 / 255.0).abs() < 1.0e-6
        }));
        assert!(!run_alphas.iter().any(|(owner, alpha)| {
            *owner == Some("caption/run/0") && (*alpha - 48.0 / 255.0).abs() < 1.0e-6
        }));
        assert!(run_alphas.iter().any(|(owner, alpha)| {
            *owner == Some("caption/run/1") && (*alpha - 24.0 / 255.0).abs() < 1.0e-6
        }));
        assert!(!run_alphas.iter().any(|(owner, alpha)| {
            *owner == Some("caption/run/1") && (*alpha - 64.0 / 255.0).abs() < 1.0e-6
        }));
    }

    #[test]
    fn compiled_shaper_preserves_rich_runs_and_wraps_block_local_presentation() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["rich ".to_owned(), "caption".to_owned()];
        let run_styles = [
            super::super::CaptionRunStyle {
                font_size: 24.0,
                color: [255, 255, 255, 255],
                font_weight: 400,
            },
            super::super::CaptionRunStyle {
                font_size: 48.0,
                color: [252, 252, 162, 255],
                font_weight: 700,
            },
        ];
        let program = semantics
            .build_caption_draw_program(
                &CaptionDrawInput {
                    runs: &runs,
                    run_styles: &run_styles,
                    font_family: crate::resource::DEFAULT_FONT_FAMILY,
                    shadow: Some(super::super::CaptionShadow {
                        color: [0, 0, 0, 230],
                        offset: [2.0, 2.0],
                        blur_sigma: 5.0,
                    }),
                    region: [0.0, 0.5, 0.5, 0.5],
                    align: [0.0, 0.0],
                    presentation: super::super::CaptionPresentation {
                        opacity: 0.75,
                        translation: [12.0, 8.0],
                        scale: 1.5,
                        rotation: 0.0,
                        clip_inset: [0.1, 0.1, 0.1, 0.1],
                        blur_sigma: 3.0,
                    },
                    behavior: CaptionBehavior::Static,
                },
                320,
                180,
            )
            .unwrap();

        let mut font_sizes = program
            .nodes()
            .iter()
            .filter_map(|node| match node {
                Node::GlyphRun(run) => Some(run.font_size),
                _ => None,
            })
            .collect::<Vec<_>>();
        font_sizes.sort_by(f32::total_cmp);
        font_sizes.dedup_by(|left, right| left.to_bits() == right.to_bits());
        assert_eq!(font_sizes, vec![24.0, 48.0]);
        assert!(
            program
                .paints()
                .contains(&Paint::Solid(linear_color([255, 255, 255, 255])))
        );
        assert!(
            program
                .paints()
                .contains(&Paint::Solid(linear_color([252, 252, 162, 255])))
        );
        assert!(program.nodes().iter().any(|node| matches!(
            node,
            Node::Group(group)
                if group.filters.iter().any(|filter| matches!(filter, Filter::DropShadow { sigma_x, sigma_y, .. } if *sigma_x == 5.0 && *sigma_y == 5.0))
        )));
        let presentation = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::Group(group) if group.opacity == 0.75 => Some(group),
                _ => None,
            })
            .expect("presentation group");
        assert!(presentation.clip.is_some());
        assert!(presentation.filters.iter().any(
            |filter| matches!(filter, Filter::Blur { sigma_x, sigma_y } if *sigma_x == 3.0 && *sigma_y == 3.0)
        ));
        let matrix = presentation.transform.0;
        assert_eq!(matrix[0], 1.5);
        assert_eq!(matrix[4], 1.5);
        let pivot_x = (12.0 - matrix[2]) / 0.5;
        let pivot_y = (8.0 - matrix[5]) / 0.5;
        assert!((0.0..160.0).contains(&pivot_x));
        assert!((90.0..180.0).contains(&pivot_y));
    }

    #[test]
    fn compiled_shaper_interprets_presentation_rotation_as_radians() {
        let mut semantics = TextSemantics::product();
        let digest =
            crate::resource::ContentDigest::of_bytes(super::super::BUNDLED_FALLBACK_FONT).as_hex();
        semantics
            .register_program_font(
                crate::resource::DEFAULT_FONT_FAMILY,
                &digest,
                0,
                Arc::new(super::super::BUNDLED_FALLBACK_FONT.to_vec()),
            )
            .unwrap();
        let runs = vec!["quarter turn".to_owned()];
        let mut caption = input(&runs);
        caption.presentation.rotation = std::f64::consts::FRAC_PI_2;
        let program = semantics
            .build_caption_draw_program(&caption, 320, 180)
            .unwrap();

        let matrix = program
            .nodes()
            .iter()
            .find_map(|node| match node {
                Node::Group(group) if group.transform != Transform2d::IDENTITY => {
                    Some(group.transform.0)
                }
                _ => None,
            })
            .expect("rotated presentation group");
        assert!(matrix[0].abs() < 1e-12, "cos(pi/2) = {}", matrix[0]);
        assert!(
            (matrix[1] + 1.0).abs() < 1e-12,
            "-sin(pi/2) = {}",
            matrix[1]
        );
        assert!((matrix[3] - 1.0).abs() < 1e-12, "sin(pi/2) = {}", matrix[3]);
        assert!(matrix[4].abs() < 1e-12, "cos(pi/2) = {}", matrix[4]);
    }
}
