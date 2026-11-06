//! Compare sequential, reverse and alternating seeks with independently loaded scene samples.

use crate::{case::Case, pixels, scene};
use std::path::Path;
use valle_engine::gpu::{Gpu, Renderer};
use valle_engine::scene::Frame;

pub fn check(
    root: &Path,
    dir: &Path,
    case: &Case,
    expected: &[Frame],
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
            let reference = &expected[i];
            let measurements =
                |f| serde_json::to_value(scene::measure(f)).map_err(|e| e.to_string());
            if actual.styles != reference.styles
                || actual.items != reference.items
                || actual.pictures != reference.pictures
                || actual.transforms != reference.transforms
                || measurements(actual)? != measurements(reference)?
            {
                return Err(format!(
                    "{label}: frame data differs from a freshly loaded scene"
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
