//! Worker-local Skottie fulfillment for Product resource requests.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use skia_safe::resources::{self, ResourceProvider};
use skia_safe::{AlphaType, Color, ColorType, Data, FontMgr, ImageInfo, Rect, Typeface, surfaces};
use valle_media::frame::RgbaFrame;

pub(super) struct LottieDocument {
    animation: skia_safe::skottie::Animation,
}

impl LottieDocument {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let json = std::fs::read_to_string(path)
            .with_context(|| format!("read Lottie JSON {}", path.display()))?;
        let provider = BundleResourceProvider {
            root: path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
            font_mgr: FontMgr::default(),
        };
        let animation = skia_safe::skottie::Builder::new()
            .set_resource_provider(provider)
            .make(&json)
            .ok_or_else(|| anyhow!("parse Lottie JSON {}", path.display()))?;
        Ok(Self { animation })
    }

    pub(super) fn render_at(
        &self,
        source_time_s: f64,
        width: u32,
        height: u32,
    ) -> Result<RgbaFrame> {
        let mut frame = RgbaFrame::new(width, height);
        let info = ImageInfo::new(
            (width as i32, height as i32),
            ColorType::RGBA8888,
            AlphaType::Premul,
            None,
        );
        {
            let mut surface =
                surfaces::wrap_pixels(&info, &mut frame.data, width as usize * 4, None)
                    .ok_or_else(|| anyhow!("wrap Lottie frame surface"))?;
            let canvas = surface.canvas();
            canvas.clear(Color::TRANSPARENT);
            self.animation.seek_frame_time(source_time_s.max(0.0));
            self.animation.render(
                canvas,
                Rect::from_xywh(0.0, 0.0, width as f32, height as f32),
            );
        }
        unpremultiply_rgba(&mut frame.data);
        Ok(frame)
    }
}

#[derive(Debug)]
struct BundleResourceProvider {
    root: PathBuf,
    font_mgr: FontMgr,
}

impl ResourceProvider for BundleResourceProvider {
    fn load(&self, resource_path: &str, resource_name: &str) -> Option<Data> {
        match resources::helpers::identify_resource_kind(resource_path, resource_name) {
            resources::helpers::ResourceKind::Base64(data) => Some(data),
            resources::helpers::ResourceKind::DownloadFromUrl(url) => {
                if url.contains("://") {
                    return None;
                }
                let path = safe_join(&self.root, Path::new(&url))?;
                std::fs::read(path).ok().map(|bytes| Data::new_copy(&bytes))
            }
        }
    }

    fn load_typeface(&self, name: &str, url: &str) -> Option<Typeface> {
        resources::helpers::load_typeface(self, &self.font_mgr, name, url)
    }

    fn font_mgr(&self) -> FontMgr {
        self.font_mgr.clone()
    }
}

fn safe_join(root: &Path, relative: &Path) -> Option<PathBuf> {
    let mut result = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(result)
}

fn unpremultiply_rgba(bytes: &mut [u8]) {
    for pixel in bytes.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}
