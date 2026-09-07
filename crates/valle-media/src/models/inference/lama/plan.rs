use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::models::inference::lama::frame::BinaryMaskView;

pub const CONTEXT_PIXELS: u32 = 32;
pub const SPARSE_FILL_THRESHOLD: f32 = 0.15;
pub const PARTITION_CORE_LIMIT: u32 = 320;

/// Fixed shapes published by the LaMa release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CanvasBucket {
    Square256,
    Wide640x384,
}

impl CanvasBucket {
    pub const ALL: [Self; 2] = [Self::Square256, Self::Wide640x384];

    pub const fn width(self) -> u32 {
        match self {
            Self::Square256 => 256,
            Self::Wide640x384 => 640,
        }
    }

    pub const fn height(self) -> u32 {
        match self {
            Self::Square256 => 256,
            Self::Wide640x384 => 384,
        }
    }

    pub const fn onnx_filename(self) -> &'static str {
        match self {
            Self::Square256 => "lama_256x256.onnx",
            Self::Wide640x384 => "lama_640x384.onnx",
        }
    }
}

/// Half-open image rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn right(self) -> u32 {
        self.x + self.width
    }

    pub const fn bottom(self) -> u32 {
        self.y + self.height
    }

    const fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStrategy {
    Noop,
    Single,
    Partitioned,
}

/// One sequential crop repair. `mask_region` limits which original mask pixels
/// belong to this task; `crop` includes the contract's 32-pixel context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InpaintTask {
    pub mask_region: Rect,
    pub crop: Rect,
}

impl InpaintTask {
    pub fn scales_for(self, bucket: CanvasBucket) -> bool {
        self.crop.width > bucket.width() || self.crop.height > bucket.height()
    }
}

/// Complete deterministic plan for one image/mask pair.
///
/// Sparse tasks are generated in row-major order. One bucket is selected for
/// all tasks so a caller can load exactly one fixed-shape model session before
/// processing an image stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InpaintPlan {
    pub image_width: u32,
    pub image_height: u32,
    pub strategy: PlanStrategy,
    pub bucket: Option<CanvasBucket>,
    pub masked_pixels: usize,
    pub mask_fill_ratio: f32,
    pub tasks: Vec<InpaintTask>,
}

pub fn plan_inpaint(mask: BinaryMaskView<'_>) -> Result<InpaintPlan> {
    let width = mask.width();
    let height = mask.height();
    let full = Rect::full(width, height);
    let masked_pixels = mask.pixels().iter().filter(|&&value| value == 255).count();
    if masked_pixels == 0 {
        return Ok(InpaintPlan {
            image_width: width,
            image_height: height,
            strategy: PlanStrategy::Noop,
            bucket: None,
            masked_pixels: 0,
            mask_fill_ratio: 0.0,
            tasks: Vec::new(),
        });
    }

    let bounds = mask_bounds(mask, full).expect("non-empty mask must have bounds");
    let bounding_area = bounds.width as usize * bounds.height as usize;
    let fill_ratio = masked_pixels as f32 / bounding_area as f32;
    let strategy = if fill_ratio < SPARSE_FILL_THRESHOLD {
        PlanStrategy::Partitioned
    } else {
        PlanStrategy::Single
    };

    let tasks = match strategy {
        PlanStrategy::Single => vec![InpaintTask {
            mask_region: full,
            crop: expand_with_context(bounds, width, height),
        }],
        PlanStrategy::Partitioned => partition_tasks(mask)?,
        PlanStrategy::Noop => unreachable!(),
    };
    ensure!(!tasks.is_empty(), "non-empty mask produced no LaMa tasks");

    let bucket = CanvasBucket::ALL
        .into_iter()
        .find(|bucket| tasks.iter().all(|task| !task.scales_for(*bucket)))
        .unwrap_or(CanvasBucket::Wide640x384);

    Ok(InpaintPlan {
        image_width: width,
        image_height: height,
        strategy,
        bucket: Some(bucket),
        masked_pixels,
        mask_fill_ratio: fill_ratio,
        tasks,
    })
}

fn partition_tasks(mask: BinaryMaskView<'_>) -> Result<Vec<InpaintTask>> {
    let fallback = CanvasBucket::Wide640x384;
    let horizontal = fallback
        .width()
        .checked_sub(2 * CONTEXT_PIXELS)
        .ok_or_else(|| anyhow::anyhow!("fallback canvas is narrower than its context"))?;
    let vertical = fallback
        .height()
        .checked_sub(2 * CONTEXT_PIXELS)
        .ok_or_else(|| anyhow::anyhow!("fallback canvas is shorter than its context"))?;
    let core = PARTITION_CORE_LIMIT.min(horizontal).min(vertical);
    ensure!(core > 0, "LaMa partition core must be positive");

    let mut tasks = Vec::new();
    let mut y = 0;
    while y < mask.height() {
        let tile_height = core.min(mask.height() - y);
        let mut x = 0;
        while x < mask.width() {
            let tile_width = core.min(mask.width() - x);
            let region = Rect {
                x,
                y,
                width: tile_width,
                height: tile_height,
            };
            if let Some(bounds) = mask_bounds(mask, region) {
                tasks.push(InpaintTask {
                    mask_region: region,
                    crop: expand_with_context(bounds, mask.width(), mask.height()),
                });
            }
            x = x
                .checked_add(core)
                .ok_or_else(|| anyhow::anyhow!("partition x position overflow"))?;
        }
        y = y
            .checked_add(core)
            .ok_or_else(|| anyhow::anyhow!("partition y position overflow"))?;
    }
    Ok(tasks)
}

fn mask_bounds(mask: BinaryMaskView<'_>, region: Rect) -> Option<Rect> {
    let image_width = mask.width() as usize;
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut found = false;

    for y in region.y..region.bottom() {
        for x in region.x..region.right() {
            if mask.pixels()[y as usize * image_width + x as usize] == 255 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    found.then_some(Rect {
        x: min_x,
        y: min_y,
        width: max_x - min_x + 1,
        height: max_y - min_y + 1,
    })
}

fn expand_with_context(bounds: Rect, image_width: u32, image_height: u32) -> Rect {
    let x0 = bounds.x.saturating_sub(CONTEXT_PIXELS);
    let y0 = bounds.y.saturating_sub(CONTEXT_PIXELS);
    let x1 = bounds
        .right()
        .saturating_add(CONTEXT_PIXELS)
        .min(image_width);
    let y1 = bounds
        .bottom()
        .saturating_add(CONTEXT_PIXELS)
        .min(image_height);
    Rect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask(width: u32, height: u32, points: &[(u32, u32)]) -> Vec<u8> {
        let mut mask = vec![0; width as usize * height as usize];
        for &(x, y) in points {
            mask[y as usize * width as usize + x as usize] = 255;
        }
        mask
    }

    #[test]
    fn empty_small_and_large_masks_route_deterministically() {
        let empty = vec![0; 640 * 384];
        let plan = plan_inpaint(BinaryMaskView::gray8(640, 384, &empty).unwrap()).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Noop);
        assert_eq!(plan.bucket, None);

        let mut small = vec![0; 640 * 384];
        for y in 100..140 {
            for x in 200..240 {
                small[y * 640 + x] = 255;
            }
        }
        let plan = plan_inpaint(BinaryMaskView::gray8(640, 384, &small).unwrap()).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Single);
        assert_eq!(plan.bucket, Some(CanvasBucket::Square256));
        assert_eq!(plan.tasks[0].crop.width, 104);
        assert_eq!(plan.tasks[0].crop.height, 104);

        let mut large = vec![0; 640 * 384];
        for y in 40..340 {
            for x in 100..540 {
                large[y * 640 + x] = 255;
            }
        }
        let plan = plan_inpaint(BinaryMaskView::gray8(640, 384, &large).unwrap()).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Single);
        assert_eq!(plan.bucket, Some(CanvasBucket::Wide640x384));
        assert!(plan.tasks[0].scales_for(CanvasBucket::Square256));
    }

    #[test]
    fn sparse_masks_are_partitioned_in_row_major_order() {
        let points = [(10, 10), (620, 10), (10, 620), (620, 620)];
        let bytes = mask(640, 640, &points);
        let plan = plan_inpaint(BinaryMaskView::gray8(640, 640, &bytes).unwrap()).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Partitioned);
        assert_eq!(plan.tasks.len(), 4);
        assert_eq!(plan.tasks[0].mask_region.x, 0);
        assert_eq!(plan.tasks[1].mask_region.x, 320);
        assert_eq!(plan.tasks[2].mask_region.y, 320);
        assert_eq!(
            plan.tasks[3].mask_region,
            Rect {
                x: 320,
                y: 320,
                width: 320,
                height: 320,
            }
        );
        assert_eq!(plan.bucket, Some(CanvasBucket::Square256));
    }

    #[test]
    fn one_fixed_bucket_covers_every_sparse_task() {
        let points = [(1, 10), (318, 10), (500, 20)];
        let bytes = mask(640, 200, &points);
        let plan = plan_inpaint(BinaryMaskView::gray8(640, 200, &bytes).unwrap()).unwrap();
        assert_eq!(plan.strategy, PlanStrategy::Partitioned);
        assert_eq!(plan.tasks.len(), 2);
        assert!(plan.tasks[0].scales_for(CanvasBucket::Square256));
        assert!(!plan.tasks[1].scales_for(CanvasBucket::Square256));
        assert_eq!(plan.bucket, Some(CanvasBucket::Wide640x384));
        assert!(
            plan.tasks
                .iter()
                .all(|task| !task.scales_for(CanvasBucket::Wide640x384))
        );
    }
}
