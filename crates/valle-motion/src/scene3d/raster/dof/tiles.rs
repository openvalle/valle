//! Conservative CoC classification for the current nearest-pixel gather contract.
use super::{DepthOfFieldState, RasterFrame, coc, effective_depth};

const EDGE: usize = 16;
const SHARP: f32 = 0.5;

pub(super) struct Activity {
    columns: usize,
    active: Vec<bool>,
}

impl Activity {
    pub(super) fn new(frame: &RasterFrame, dof: DepthOfFieldState, far: f32) -> Self {
        let width = frame.width as usize;
        let height = frame.height as usize;
        let columns = width.div_ceil(EDGE);
        let rows = height.div_ceil(EDGE);
        let mut ranges = vec![[f32::INFINITY, 0.0_f32]; columns * rows];
        for (y, row) in frame.depth.chunks_exact(width).enumerate() {
            for (x, depths) in row.chunks(EDGE).enumerate() {
                let range = &mut ranges[(y / EDGE) * columns + x];
                for &depth in depths {
                    let depth = effective_depth(depth, far);
                    range[0] = range[0].min(depth);
                    range[1] = range[1].max(depth);
                }
            }
        }
        // CoC decreases up to focus and increases after it. Only the extrema need divisions;
        // keeping transparent/clear depths in these bounds is conservative as well.
        let maxima: Vec<_> = ranges
            .into_iter()
            .map(|[min, max]| {
                let near = if min < dof.focus_distance {
                    coc(dof, min)
                } else {
                    0.0
                };
                [coc(dof, min).max(coc(dof, max)), near]
            })
            .collect();
        // Includes the fixed one-pixel near ring and all rounding at tile borders. Expanding
        // whole tiles can overestimate support, never omit a possible foreground contributor.
        let halo = (dof.max_blur_radius.max(1.0).ceil() as usize + 1).div_ceil(EDGE);
        let active = (0..rows * columns)
            .map(|index| {
                if maxima[index][0] >= SHARP {
                    return true;
                }
                let (x, y) = (index % columns, index / columns);
                (y.saturating_sub(halo)..=(y + halo).min(rows - 1)).any(|sy| {
                    (x.saturating_sub(halo)..=(x + halo).min(columns - 1))
                        .any(|sx| maxima[sy * columns + sx][1] >= SHARP)
                })
            })
            .collect();
        Self { columns, active }
    }

    pub(super) fn any(&self) -> bool {
        self.active.iter().any(|&v| v)
    }

    pub(super) fn at(&self, x: usize, y: usize) -> bool {
        self.active[y / EDGE * self.columns + x / EDGE]
    }
}

// If own CoC < 0.5, the main gather's 0.75 threshold cannot be reached. A near tap at the
// center fails its depth test; every other nearest-pixel tap is at least one pixel away.
// With all possible foreground CoCs < 0.5, none can contribute. Returning original F16
// bits is therefore exact for this sampler, not a claim about future bilinear kernels.
