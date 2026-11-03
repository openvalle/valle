//! Color glyphs: a glyph a font paints in colors by its COLR table (emoji), rasterized on the CPU
//! by vello_cpu at the size it shows at. A font of colors by other tables only (sbix, CBDT, SVG)
//! is an error.

use std::sync::Arc;

use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
use vello_cpu::color::AlphaColor;
use vello_cpu::kurbo::Affine;
use vello_cpu::peniko::FontData;
use vello_cpu::{Level, Pixmap, RenderContext, RenderSettings, Resources};

use crate::image::Image;
use crate::template::Error;
use crate::text::Face;

/// Whether glyph `id` of `face` has colors of its own.
pub fn colored(face: &Face, id: u32) -> Result<bool, Error> {
    let font = FontRef::from_index(face.data.data(), face.index).map_err(|e| e.to_string())?;
    if font.color_glyphs().get(GlyphId::new(id)).is_some() {
        return Ok(true);
    }
    let has = |tag: &[u8; 4]| font.table_data(Tag::new(tag)).is_some();
    if !has(b"COLR") && [b"sbix", b"CBDT", b"SVG "].iter().any(|t| has(t)) {
        return Err(
            "a font of color glyphs by its sbix, CBDT or SVG table, which are not \
             supported: use one with a COLR table"
                .into(),
        );
    }
    Ok(false)
}

/// Glyph `id` of `face` in its colors at `ppem` px an em, `foreground` (straight sRGB) where it
/// paints in the text's color: premultiplied pixels cropped to what it paints, and their top left
/// from its origin in px, y down. Nothing for a glyph that paints nothing, or more than 4096 px
/// across.
pub fn raster(face: &Face, id: u32, ppem: f32, foreground: [u8; 3]) -> Option<(Image, [f32; 2])> {
    let font = FontRef::from_index(face.data.data(), face.index).ok()?;
    let coords: Vec<NormalizedCoord> = (face.coords.iter())
        .map(|&c| NormalizedCoord::from_bits(c))
        .collect();
    let location = LocationRef::new(&coords);
    let size = Size::new(ppem);
    let metrics = font.metrics(size, location);
    let advance = (font.glyph_metrics(size, location))
        .advance_width(GlyphId::new(id))
        .unwrap_or(ppem);
    // The em box and the advance, with room for what reaches past them and for a slant.
    let pad = (ppem * 0.5).ceil() + 2.0;
    let origin = [pad, metrics.ascent.ceil() + pad];
    let width = (advance + 2.0 * pad).ceil();
    let height = (metrics.ascent - metrics.descent + 2.0 * pad).ceil() + 1.0;
    if !(1.0..=4096.0).contains(&width) || !(1.0..=4096.0).contains(&height) {
        return None;
    }
    let (w, h) = (width as u16, height as u16);
    // The same pixels on every host: no SIMD level picked at run time.
    let settings = RenderSettings {
        level: Level::fallback(),
        num_threads: 0,
    };
    let mut ctx = RenderContext::new_with(w, h, settings);
    let mut resources = Resources::new();
    let [r, g, b] = foreground;
    ctx.set_paint(AlphaColor::from_rgba8(r, g, b, 255));
    let data = FontData::new(face.data.clone(), face.index);
    // Slanted as the outlines are for an italic the face does not have.
    let slant = Affine::skew(if face.slanted { -0.25 } else { 0.0 }, 0.0);
    ctx.glyph_run(&mut resources, &data)
        .font_size(ppem)
        .hint(false)
        .normalized_coords(&face.coords)
        .glyph_transform(slant)
        .fill_glyphs(std::iter::once(vello_cpu::Glyph {
            id,
            x: origin[0],
            y: origin[1],
        }))
        .ok()?;
    ctx.flush();
    let mut pixmap = Pixmap::new(w, h);
    ctx.render(&mut pixmap, &mut resources);
    let px = pixmap.data_as_u8_slice();
    let (w, h) = (usize::from(w), usize::from(h));
    // What it paints.
    let (mut low, mut high) = ([w, h], [0, 0]);
    for y in 0..h {
        for x in 0..w {
            if px[(y * w + x) * 4 + 3] > 0 {
                low = [low[0].min(x), low[1].min(y)];
                high = [high[0].max(x + 1), high[1].max(y + 1)];
            }
        }
    }
    if high[0] <= low[0] {
        return None;
    }
    let mut pixels = Vec::with_capacity((high[0] - low[0]) * (high[1] - low[1]) * 4);
    for y in low[1]..high[1] {
        pixels.extend_from_slice(&px[(y * w + low[0]) * 4..(y * w + high[0]) * 4]);
    }
    let image = Image {
        size: [(high[0] - low[0]) as u32, (high[1] - low[1]) as u32],
        pixels: Arc::from(pixels),
    };
    Some((
        image,
        [low[0] as f32 - origin[0], low[1] as f32 - origin[1]],
    ))
}
