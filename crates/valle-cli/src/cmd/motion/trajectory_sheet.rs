//! Render the H03 review sheet from the same frozen Motion package as native delivery.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{Context, Result, anyhow};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use serde_json::{Value, json};
use valle_engine::render::FrameKey;
use valle_motion::{MotionTrajectory, SceneArtifact};
use valle_render::host::{
    NativeProject, NativeRenderOptions, NativeRenderer, NativeResourceCatalog,
};

use super::{BoundAsset, Delivery, fixed_package_font_blobs};
use crate::cmd::motion_package::{StandaloneMotionPackageInput, build_standalone_motion_package};

const COLORS: [[u8; 3]; 8] = [
    [0, 220, 255],
    [255, 112, 112],
    [255, 210, 36],
    [107, 255, 145],
    [199, 139, 255],
    [255, 154, 66],
    [76, 166, 255],
    [255, 109, 208],
];

#[allow(clippy::too_many_arguments)]
pub(super) fn render(
    output: &Path,
    artifact: &SceneArtifact,
    assets: &BTreeMap<String, BoundAsset>,
    explicit_fonts: &[std::sync::Arc<[u8]>],
    prop_bindings: &BTreeMap<String, Value>,
    trajectories: &[MotionTrajectory],
    delivery: &Delivery,
) -> Result<Value> {
    if !output
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    {
        return Err(anyhow!("--trajectory-sheet requires a .png output path"));
    }
    super::super::fixed_render::require_new_output(output)?;
    let fonts = fixed_package_font_blobs(artifact, explicit_fonts)?;
    let package = build_standalone_motion_package(StandaloneMotionPackageInput {
        artifact,
        assets,
        font_blobs: &fonts,
        prop_bindings,
        duration: delivery.duration,
        frame_rate: delivery.fps,
        canvas: delivery.canvas.tuple(),
    })?;

    let temp = tempfile::tempdir().context("prepare review sheet resources")?;
    let mut catalog = NativeResourceCatalog::new();
    for bytes in &fonts {
        catalog.admit_bytes(bytes.clone());
    }
    for asset in assets.values() {
        let path = temp.path().join(asset.hash.as_hex());
        std::fs::write(&path, &asset.bytes)?;
        catalog.insert_file(asset.hash, path);
    }
    let project = NativeProject::from_render(package.opened.engine_render(), Arc::new(catalog));
    let (cell_width, cell_height) = cell_size(delivery.canvas.width, delivery.canvas.height);
    let renderer = NativeRenderer::new(
        project,
        NativeRenderOptions {
            output_size: Some((cell_width, cell_height)),
            ..NativeRenderOptions::default()
        },
    );
    let frame_numbers = sampled_frames(delivery.duration_frames);
    let frame_keys = frame_numbers
        .iter()
        .copied()
        .map(|frame| FrameKey::new(i64::from(frame)))
        .collect::<Vec<_>>();
    let columns = frame_numbers.len().min(4) as u32;
    let raw = temp.path().join("review-raw.png");
    renderer.render_png_frames(&frame_keys, &[], Some((&raw, columns)))?;
    let mut sheet = image::open(&raw)
        .with_context(|| format!("opening review sheet {}", raw.display()))?
        .into_rgba8();
    let mut colors = BTreeMap::new();
    for (track_index, track) in trajectories.iter().enumerate() {
        let color = COLORS[track_index % COLORS.len()];
        colors.insert(
            track.node.clone(),
            format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]),
        );
        for (cell_index, frame) in frame_numbers.iter().copied().enumerate() {
            let origin = (
                (cell_index as u32 % columns) * cell_width,
                (cell_index as u32 / columns) * cell_height,
            );
            draw_track(
                &mut sheet,
                origin,
                (cell_width, cell_height),
                delivery.canvas.tuple(),
                track,
                frame,
                color,
            );
        }
    }
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o666));
    }
    let mut staged = builder
        .tempfile_in(parent)
        .with_context(|| format!("staging review sheet in {}", parent.display()))?;
    DynamicImage::ImageRgba8(sheet).write_to(&mut staged, ImageFormat::Png)?;
    staged.persist_noclobber(output).map_err(|error| {
        anyhow!(
            "publishing review sheet {}: {}",
            output.display(),
            error.error
        )
    })?;
    Ok(json!({
        "path": output,
        "frames": frame_numbers,
        "columns": columns,
        "cellSize": [cell_width, cell_height],
        "colors": colors,
    }))
}

fn cell_size(width: u32, height: u32) -> (u32, u32) {
    const LONG_EDGE: u32 = 320;
    if width >= height {
        (
            LONG_EDGE,
            ((u64::from(LONG_EDGE) * u64::from(height)) / u64::from(width)).max(1) as u32,
        )
    } else {
        (
            ((u64::from(LONG_EDGE) * u64::from(width)) / u64::from(height)).max(1) as u32,
            LONG_EDGE,
        )
    }
}

fn sampled_frames(duration: u32) -> Vec<u32> {
    let cells = duration.min(12);
    (0..cells)
        .map(|index| {
            if cells == 1 {
                0
            } else {
                ((u64::from(index) * u64::from(duration - 1)) / u64::from(cells - 1)) as u32
            }
        })
        .collect()
}

fn draw_track(
    sheet: &mut RgbaImage,
    origin: (u32, u32),
    cell: (u32, u32),
    canvas: (u32, u32),
    track: &MotionTrajectory,
    frame: u32,
    color: [u8; 3],
) {
    let to_cell = |position: [f64; 2]| {
        (
            position[0] * f64::from(cell.0) / f64::from(canvas.0),
            position[1] * f64::from(cell.1) / f64::from(canvas.1),
        )
    };
    for pair in track.points.windows(2) {
        if pair[1].frame != pair[0].frame + 1 {
            continue;
        }
        let Some((start, end)) =
            clip_line(to_cell(pair[0].position), to_cell(pair[1].position), cell)
        else {
            continue;
        };
        let alpha = if pair[1].frame <= frame { 220 } else { 100 };
        draw_line(sheet, origin, cell, start, end, color, alpha);
    }
    if let Some(point) = track.points.iter().find(|point| point.frame == frame) {
        let at = to_cell(point.position);
        if at.0 >= 0.0 && at.0 < f64::from(cell.0) && at.1 >= 0.0 && at.1 < f64::from(cell.1) {
            stamp(
                sheet,
                origin,
                cell,
                at.0.round() as i32,
                at.1.round() as i32,
                [0, 0, 0],
                255,
                5,
            );
            stamp(
                sheet,
                origin,
                cell,
                at.0.round() as i32,
                at.1.round() as i32,
                color,
                255,
                3,
            );
        }
    }
}

fn draw_line(
    sheet: &mut RgbaImage,
    origin: (u32, u32),
    cell: (u32, u32),
    start: (f64, f64),
    end: (f64, f64),
    color: [u8; 3],
    alpha: u8,
) {
    let steps = (end.0 - start.0).abs().max((end.1 - start.1).abs()).ceil() as u32;
    for step in 0..=steps {
        let t = if steps == 0 {
            0.0
        } else {
            f64::from(step) / f64::from(steps)
        };
        let x = (start.0 + (end.0 - start.0) * t).round() as i32;
        let y = (start.1 + (end.1 - start.1) * t).round() as i32;
        stamp(sheet, origin, cell, x, y, [0, 0, 0], alpha / 2, 2);
        stamp(sheet, origin, cell, x, y, color, alpha, 1);
    }
}

fn stamp(
    sheet: &mut RgbaImage,
    origin: (u32, u32),
    cell: (u32, u32),
    x: i32,
    y: i32,
    color: [u8; 3],
    alpha: u8,
    radius: i32,
) {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= cell.0 as i32 || py >= cell.1 as i32 {
                continue;
            }
            let pixel = sheet.get_pixel_mut(origin.0 + px as u32, origin.1 + py as u32);
            let old = *pixel;
            let a = f32::from(alpha) / 255.0;
            *pixel = Rgba([
                (f32::from(color[0]) * a + f32::from(old[0]) * (1.0 - a)).round() as u8,
                (f32::from(color[1]) * a + f32::from(old[1]) * (1.0 - a)).round() as u8,
                (f32::from(color[2]) * a + f32::from(old[2]) * (1.0 - a)).round() as u8,
                (f32::from(alpha) + f32::from(old[3]) * (1.0 - a)).round() as u8,
            ]);
        }
    }
}

/// Liang-Barsky clipping keeps long off-canvas trajectories from creating unbounded line loops.
fn clip_line(
    start: (f64, f64),
    end: (f64, f64),
    cell: (u32, u32),
) -> Option<((f64, f64), (f64, f64))> {
    if ![start.0, start.1, end.0, end.1]
        .into_iter()
        .all(f64::is_finite)
    {
        return None;
    }
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let mut low: f64 = 0.0;
    let mut high: f64 = 1.0;
    for (p, q) in [
        (-dx, start.0),
        (dx, f64::from(cell.0 - 1) - start.0),
        (-dy, start.1),
        (dy, f64::from(cell.1 - 1) - start.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let r = q / p;
        if p < 0.0 {
            low = low.max(r);
        } else {
            high = high.min(r);
        }
        if low > high {
            return None;
        }
    }
    Some((
        (start.0 + low * dx, start.1 + low * dy),
        (start.0 + high * dx, start.1 + high * dy),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_first_and_last_frame_and_clips_large_segments() {
        assert_eq!(sampled_frames(1), vec![0]);
        assert_eq!(
            sampled_frames(60),
            vec![0, 5, 10, 16, 21, 26, 32, 37, 42, 48, 53, 59]
        );
        let clipped = clip_line((-1000.0, 5.0), (1000.0, 5.0), (20, 10)).unwrap();
        for (actual, expected) in [
            (clipped.0.0, 0.0),
            (clipped.0.1, 5.0),
            (clipped.1.0, 19.0),
            (clipped.1.1, 5.0),
        ] {
            assert!((actual - expected).abs() < 1e-10);
        }
        assert!(clip_line((-20.0, -10.0), (-1.0, -1.0), (20, 10)).is_none());
    }
}
