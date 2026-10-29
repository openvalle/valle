//! Record shared SVG geometry once; draw it at the destination resolution on either backend.
use skia_safe::{self as sk, canvas::SaveLayerRec};
use valle_engine::motion::svg::{Op, Paint as SvgPaint, SvgDocument};

pub(super) fn picture(doc: &SvgDocument) -> Result<sk::Picture, String> {
    record(&doc.ops, doc.width, doc.height, true)
}
fn matrix([a, b, c, d, e, f]: [f32; 6]) -> sk::Matrix {
    sk::Matrix::new_all(a, c, e, b, d, f, 0., 0., 1.)
}
fn color([r, g, b]: [u8; 3], a: f32) -> sk::Color {
    sk::Color::from_argb((a * 255.).round() as u8, r, g, b)
}
fn mode(s: &str) -> Result<sk::BlendMode, String> {
    Ok(match s {
        "Normal" => sk::BlendMode::SrcOver,
        "Multiply" => sk::BlendMode::Multiply,
        "Screen" => sk::BlendMode::Screen,
        "Overlay" => sk::BlendMode::Overlay,
        "Darken" => sk::BlendMode::Darken,
        "Lighten" => sk::BlendMode::Lighten,
        "ColorDodge" => sk::BlendMode::ColorDodge,
        "ColorBurn" => sk::BlendMode::ColorBurn,
        "HardLight" => sk::BlendMode::HardLight,
        "SoftLight" => sk::BlendMode::SoftLight,
        "Difference" => sk::BlendMode::Difference,
        "Exclusion" => sk::BlendMode::Exclusion,
        "Hue" => sk::BlendMode::Hue,
        "Saturation" => sk::BlendMode::Saturation,
        "Color" => sk::BlendMode::Color,
        "Luminosity" => sk::BlendMode::Luminosity,
        "PlusLighter" => sk::BlendMode::Plus,
        _ => return Err(format!("unsupported SVG blend {s}")),
    })
}
fn record(ops: &[Op], w: f32, h: f32, working: bool) -> Result<sk::Picture, String> {
    let mut recorder = sk::PictureRecorder::new();
    let canvas = recorder.begin_recording(sk::Rect::from_wh(w, h), true);
    canvas.clip_rect(sk::Rect::from_wh(w, h), None, true);
    // An SVG image is one isolated source; internal blend/mask operators must never see
    // a caller's backdrop, even when the executor replays it directly for sharp coverage.
    canvas.save_layer(&SaveLayerRec::default().bounds(&sk::Rect::from_wh(w, h)));
    draw(canvas, ops, working)?;
    canvas.restore();
    recorder
        .finish_recording_as_picture(None)
        .ok_or_else(|| "SVG recording failed".into())
}
fn path(s: &str, evenodd: bool) -> Result<sk::Path, String> {
    let mut p = if s.is_empty() {
        sk::Path::new()
    } else {
        sk::Path::from_svg(s).ok_or("invalid SVG path")?
    };
    p.set_fill_type(if evenodd {
        sk::PathFillType::EvenOdd
    } else {
        sk::PathFillType::Winding
    });
    Ok(p)
}
fn paint(source: &SvgPaint, opacity: f32, working: bool) -> Result<sk::Paint, String> {
    let mut p = sk::Paint::default();
    p.set_anti_alias(true);
    p.set_alpha_f(opacity);
    let shader = match source {
        SvgPaint::Color { rgb } => {
            if working {
                let value = valle_draw::program::AuthorColor::from_srgb8(valle_draw::Rgba::rgb(
                    rgb[0], rgb[1], rgb[2],
                ))
                .to_working();
                let value = valle_draw::program::LinearColor::new(
                    value.red * opacity,
                    value.green * opacity,
                    value.blue * opacity,
                    opacity,
                );
                super::effect::set_working_color(&mut p, value).map_err(|e| e.to_string())?;
            } else {
                p.set_color(color(*rgb, opacity));
            }
            return Ok(p);
        }
        SvgPaint::Linear {
            start,
            end,
            gradient: g,
        }
        | SvgPaint::Radial {
            center: start,
            focal: end,
            gradient: g,
            ..
        } => {
            let colors: Vec<_> = g
                .stops
                .iter()
                .map(|s| sk::Color4f::from(color(s.rgb, s.opacity)))
                .collect();
            let positions: Vec<_> = g.stops.iter().map(|s| s.offset).collect();
            let tile = match g.spread.as_str() {
                "repeat" => sk::TileMode::Repeat,
                "mirror" => sk::TileMode::Mirror,
                _ => sk::TileMode::Clamp,
            };
            let m = matrix(g.matrix);
            let pt = |v: &[f32; 2]| (v[0], v[1]);
            let gradient = sk::gradient::Gradient::new(
                sk::gradient::Colors::new(
                    colors.as_slice(),
                    Some(positions.as_slice()),
                    tile,
                    None,
                ),
                sk::gradient::Interpolation {
                    color_space: sk::gradient::interpolation::ColorSpace::SRGB,
                    ..Default::default()
                },
            );
            if let SvgPaint::Radial { radius, .. } = source {
                sk::gradient::shaders::two_point_conical_gradient(
                    (pt(end), 0.),
                    (pt(start), *radius),
                    &gradient,
                    &m,
                )
            } else {
                sk::gradient::shaders::linear_gradient((pt(start), pt(end)), &gradient, &m)
            }
        }
        SvgPaint::Pattern {
            ops,
            matrix: m,
            width,
            height,
        } => Some(record(ops, *width, *height, working)?.to_shader(
            (sk::TileMode::Repeat, sk::TileMode::Repeat),
            sk::FilterMode::Linear,
            &matrix(*m),
            &sk::Rect::from_wh(*width, *height),
        )),
    }
    .ok_or("SVG paint shader failed")?;
    p.set_shader(shader);
    Ok(p)
}
fn draw(canvas: &sk::Canvas, ops: &[Op], working: bool) -> Result<(), String> {
    let mut stack: Vec<Option<(&[Op], bool)>> = Vec::new();
    for op in ops {
        match op {
            Op::Transform { matrix: m } => {
                canvas.save();
                canvas.concat(&matrix(*m));
                stack.push(None)
            }
            Op::Clip { path: p, evenodd } => {
                canvas.save();
                canvas.clip_path(&path(p, *evenodd)?, None, true);
                stack.push(None)
            }
            Op::Opacity { opacity } => {
                canvas.save_layer_alpha_f(None, *opacity);
                stack.push(None)
            }
            Op::Blend { mode: m } => {
                let mut p = sk::Paint::default();
                p.set_blend_mode(mode(m)?);
                canvas.save_layer(&SaveLayerRec::default().paint(&p));
                stack.push(None)
            }
            Op::Mask { ops, luminance } => {
                canvas.save_layer(&SaveLayerRec::default());
                stack.push(Some((ops, *luminance)))
            }
            Op::Pop => {
                if let Some((mask, luminance)) = stack.pop().ok_or("SVG group underflow")? {
                    let mut p = sk::Paint::default();
                    p.set_blend_mode(sk::BlendMode::DstIn);
                    if luminance {
                        p.set_color_filter(sk::ColorFilter::luma());
                    }
                    canvas.save_layer(&SaveLayerRec::default().paint(&p));
                    draw(canvas, mask, working)?;
                    canvas.restore();
                }
                canvas.restore();
            }
            Op::Draw {
                path: p,
                fill,
                stroke,
            } => {
                if let Some(f) = fill {
                    canvas.draw_path(&path(p, f.evenodd)?, &paint(&f.paint, f.opacity, working)?);
                }
                if let Some(s) = stroke {
                    let mut pnt = paint(&s.paint, s.opacity, working)?;
                    pnt.set_style(sk::PaintStyle::Stroke)
                        .set_stroke_width(s.width)
                        .set_stroke_miter(s.miter);
                    pnt.set_stroke_cap(match s.cap.as_str() {
                        "Round" => sk::PaintCap::Round,
                        "Square" => sk::PaintCap::Square,
                        _ => sk::PaintCap::Butt,
                    });
                    pnt.set_stroke_join(match s.join.as_str() {
                        "Round" => sk::PaintJoin::Round,
                        "Bevel" => sk::PaintJoin::Bevel,
                        _ => sk::PaintJoin::Miter,
                    });
                    if let Some((array, offset)) = &s.dash {
                        pnt.set_path_effect(sk::PathEffect::dash(array, *offset));
                    }
                    canvas.draw_path(&path(p, false)?, &pnt);
                }
            }
            Op::Raster {
                rgba,
                width,
                height,
                rect: [x, y, w, h],
            } => {
                let info = sk::ImageInfo::new(
                    (*width as i32, *height as i32),
                    sk::ColorType::RGBA8888,
                    sk::AlphaType::Unpremul,
                    sk::ColorSpace::new_srgb(),
                );
                let image = sk::images::raster_from_data(
                    &info,
                    sk::Data::new_copy(rgba),
                    *width as usize * 4,
                )
                .ok_or("SVG raster fallback failed")?;
                canvas.draw_image_rect_with_sampling_options(
                    &image,
                    None,
                    sk::Rect::from_xywh(*x, *y, *w, *h),
                    sk::SamplingOptions::new(sk::FilterMode::Linear, sk::MipmapMode::None),
                    &sk::Paint::default(),
                );
            }
        }
    }
    if !stack.is_empty() {
        return Err("SVG groups are unbalanced".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_vector_operations_draw_pixels_and_export_shared_fixture() {
        let source = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../valle-motion/tests/fixtures/vector-image.svg"
        ));
        let doc = valle_engine::motion::svg::compile(source, &[]).unwrap();
        let picture = record(&doc.ops, doc.width, doc.height, false).unwrap();
        let info = sk::ImageInfo::new(
            (256, 256),
            sk::ColorType::RGBA8888,
            sk::AlphaType::Premul,
            sk::ColorSpace::new_srgb(),
        );
        let mut surface = sk::surfaces::raster(&info, None, None).unwrap();
        surface.canvas().clear(sk::Color::TRANSPARENT);
        surface.canvas().scale((4., 4.));
        surface.canvas().draw_picture(&picture, None, None);
        let mut pixels = vec![0u8; 256 * 256 * 4];
        assert!(surface.read_pixels(&info, &mut pixels, 256 * 4, (0, 0)));
        let pixel = |x: usize, y: usize| &pixels[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4];
        assert!(
            pixel(4, 4)[0] > 240 && pixel(124, 4)[2] > 240,
            "gradient endpoints"
        );
        assert_eq!(pixel(140, 20), [0, 255, 0, 255], "luminance mask white");
        assert_eq!(pixel(220, 20), [0, 0, 0, 0], "luminance mask black");
        assert_eq!(
            pixel(180, 180),
            [0, 255, 255, 255],
            "currentColor + circle clip"
        );
        assert_eq!(pixel(130, 130), [0, 0, 0, 0], "clip outside");
        assert_eq!(
            pixel(24, 152),
            [128, 102, 0, 128],
            "pattern + group opacity"
        );
        let working = super::picture(&doc).unwrap();
        let working_info = super::super::surface::working_info(
            valle_engine::resource::Extent2d::new(256, 256).unwrap(),
        )
        .unwrap();
        let mut working_surface = sk::surfaces::raster(&working_info, None, None).unwrap();
        working_surface.canvas().clear(sk::Color::TRANSPARENT);
        working_surface.canvas().scale((4., 4.));
        working_surface.canvas().draw_picture(&working, None, None);
        let mut working_pixels = vec![0u8; pixels.len()];
        assert!(working_surface.read_pixels(&info, &mut working_pixels, 256 * 4, (0, 0)));
        assert_eq!(
            working_pixels[(20 * 256 + 220) * 4 + 3],
            0,
            "mask must preserve transparency on working F16 targets"
        );
        working_surface.canvas().clear(sk::Color::RED);
        working_surface.canvas().draw_picture(&working, None, None);
        assert!(working_surface.read_pixels(&info, &mut working_pixels, 256 * 4, (0, 0)));
        assert_eq!(
            &working_pixels[(20 * 256 + 220) * 4..(20 * 256 + 220) * 4 + 4],
            &[255, 0, 0, 255],
            "SVG mask must not erase the caller backdrop"
        );
        if let Ok(directory) = std::env::var("VALLE_SVG_TEST_OUTPUT") {
            std::fs::write(
                format!("{directory}/svg-fixture.json"),
                serde_json::to_vec(&doc).unwrap(),
            )
            .unwrap();
            std::fs::write(format!("{directory}/svg-reference.rgba"), pixels).unwrap();
        }
    }
}
