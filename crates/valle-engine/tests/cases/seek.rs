//! Compare sequential, reverse and alternating seeks with independently loaded scene samples.

use crate::{case::Case, pixels, scene};
use std::path::Path;
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::{paint::Item, scene::Frame, text::Face};

pub struct Snapshot {
    pub frame: Frame,
    pub faces: Vec<Face>,
}

pub fn check(
    root: &Path,
    dir: &Path,
    case: &Case,
    expected: &[Snapshot],
    gpu: Option<&Gpu>,
    out: &Path,
) -> Result<usize, String> {
    let times = case.times()?;
    let mut sample = scene::load(root, &case.input(dir), times[0])?;
    let mut checker = scene::load(root, &case.input(dir), times[0])?
        .scene
        .into_checker();
    let [width, height] = sample.size();
    let mut renderer = gpu.map(|gpu| Renderer::new(gpu, width, height));
    let forward: Vec<_> = (0..times.len()).collect();
    let reverse: Vec<_> = forward.iter().rev().copied().collect();
    let alternating: Vec<_> = (0..times.len())
        .map(|i| {
            if i % 2 == 0 {
                times.len() - 1 - i / 2
            } else {
                i / 2
            }
        })
        .collect();
    let mut visits = 0;
    for (order, indices) in [
        ("forward", forward),
        ("reverse", reverse),
        ("alternating", alternating),
    ] {
        for i in indices {
            let at = times[i];
            sample.at = at;
            let number = sample.number();
            let label = format!("{order} seek to frame {number} at {}s", at.seconds_text());
            checker
                .check(at)
                .map_err(|e| format!("{label}: checker: {e}"))?;
            sample.frame = sample
                .scene
                .frame(at)
                .map_err(|e| format!("{label}: {e}"))?;
            let actual = &sample.frame;
            let reference = &expected[i].frame;
            let mut faces = Vec::new();
            let reference_items = normalized(&reference.items, &expected[i].faces, &mut faces)?;
            let actual_items = normalized(&actual.items, sample.scene.faces(), &mut faces)?;
            let pictures = |f: &Frame, source: &[Face], faces: &mut Vec<Face>| {
                f.pictures
                    .iter()
                    .map(|(id, items)| Ok((*id, normalized(items, source, faces)?)))
                    .collect::<Result<Vec<_>, String>>()
            };
            let reference_pictures = pictures(reference, &expected[i].faces, &mut faces)?;
            let actual_pictures = pictures(actual, sample.scene.faces(), &mut faces)?;
            let measurements =
                |f| serde_json::to_value(scene::measure(f)).map_err(|e| e.to_string());
            if actual.styles != reference.styles
                || actual_items != reference_items
                || actual_pictures != reference_pictures
                || actual.transforms != reference.transforms
                || measurements(actual)? != measurements(reference)?
            {
                return Err(format!(
                    "{label}: frame data differs from a freshly loaded scene (styles={}, items={}, pictures={}, transforms={}, measurements={})",
                    actual.styles != reference.styles,
                    actual_items != reference_items,
                    actual_pictures != reference_pictures,
                    actual.transforms != reference.transforms,
                    measurements(actual)? != measurements(reference)?
                ));
            }
            if let Some(renderer) = &mut renderer {
                let drawn = renderer.render(
                    &actual.items,
                    &actual.pictures,
                    sample.scene.faces(),
                    sample.scene.images(),
                )?;
                let reference = pixels::read_png(
                    &out.join(number.to_string()).join("actual.png"),
                    [width, height],
                )?;
                let (difference, _, diff) = pixels::compare(&drawn, &reference, &[])?;
                if difference.changed != 0 {
                    pixels::png(
                        &out.join(format!("seek-{order}-{number}.png")),
                        [width, height],
                        &drawn,
                    )?;
                    pixels::png(
                        &out.join(format!("seek-{order}-{number}.diff.png")),
                        [width, height],
                        &diff,
                    )?;
                    return Err(format!(
                        "{label}: {} pixels differ from the fresh render (max {})",
                        difference.changed, difference.max
                    ));
                }
            }
            visits += 1;
        }
    }
    Ok(visits)
}

// Face indices are local cache handles: a backward seek may discover the same
// fonts in another order. Compare actual font bytes/axes/synthesis, retaining
// every other display-list field and the exact rendered-pixel check.
fn normalized(items: &[Item], source: &[Face], faces: &mut Vec<Face>) -> Result<Vec<Item>, String> {
    let mut items = items.to_vec();
    for item in &mut items {
        match item {
            Item::Glyphs { face, .. } => {
                let value = source
                    .get(*face as usize)
                    .ok_or("invalid glyph face index")?;
                let index = faces
                    .iter()
                    .position(|f| {
                        f.data.data() == value.data.data()
                            && f.index == value.index
                            && f.coords == value.coords
                            && f.slanted == value.slanted
                            && f.emboldened == value.emboldened
                    })
                    .unwrap_or_else(|| {
                        faces.push(value.clone());
                        faces.len() - 1
                    });
                *face = index as u32;
            }
            Item::Group { items, .. }
            | Item::Pattern { items, .. }
            | Item::Transform { items, .. }
            | Item::Filter { items, .. }
            | Item::Cut { items, .. }
            | Item::Clip { items, .. }
            | Item::Shadow { items, .. } => *items = normalized(items, source, faces)?,
            Item::Mask { mask, items } => {
                *mask = normalized(mask, source, faces)?;
                *items = normalized(items, source, faces)?;
            }
            Item::FilterGraph { images, items, .. } => {
                for (_, input) in images {
                    *input = normalized(input, source, faces)?;
                }
                *items = normalized(items, source, faces)?;
            }
            Item::Shader { layers, .. } => {
                for (_, items) in layers {
                    *items = normalized(items, source, faces)?;
                }
            }
            Item::Rects(_)
            | Item::Paths { .. }
            | Item::Rect { .. }
            | Item::Ring { .. }
            | Item::Gradient { .. }
            | Item::Image { .. }
            | Item::Path { .. }
            | Item::Backdrop { .. } => {}
        }
    }
    Ok(items)
}

#[test]
fn seeks_compare_font_content_instead_of_cache_indices() {
    use valle_engine::text::Blob;
    let a = Face {
        data: Blob::from(vec![1u8]),
        index: 0,
        coords: vec![0],
        slanted: false,
        emboldened: false,
    };
    let b = Face {
        coords: vec![4096],
        ..a.clone()
    };
    let glyph = |face| Item::Glyphs {
        face,
        size: 16.0,
        color: [1.0; 4],
        stroke: 0.0,
        glyphs: vec![(1, [2.0, 3.0])],
    };
    let mut faces = Vec::new();
    let expected = normalized(&[glyph(0)], &[a.clone(), b.clone()], &mut faces).unwrap();
    assert_eq!(
        expected,
        normalized(&[glyph(1)], &[b.clone(), a.clone()], &mut faces).unwrap()
    );
    assert_ne!(expected, normalized(&[glyph(0)], &[b], &mut faces).unwrap());
    assert!(normalized(&[glyph(1)], &[a], &mut faces).is_err());
}
