//! Deterministic greedy rectangle placement for prepared labels.
//!
//! It knows only anchor points, rectangle sizes, priorities and bounds. Text shaping and drawing
//! stay outside this module; callers should feed measured sizes when exact typography matters.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub point: [f64; 2],
    pub size: [f64; 2],
    pub priority: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub visible: bool,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

pub fn place(candidates: &[Candidate], bounds: [f64; 4], padding: f64) -> Vec<Placement> {
    let mut order = (0..candidates.len()).collect::<Vec<_>>();
    order.sort_by(|&a, &b| {
        candidates[b]
            .priority
            .total_cmp(&candidates[a].priority)
            .then(a.cmp(&b))
    });
    let mut placed = Vec::<Rect>::new();
    let mut output = vec![
        Placement {
            x: 0.0,
            y: 0.0,
            visible: false
        };
        candidates.len()
    ];
    for index in order {
        let candidate = candidates[index];
        let [x, y] = candidate.point;
        let [w, h] = candidate.size;
        let gap = padding.max(0.0);
        let origins = [
            (x - w / 2.0, y - h - gap),
            (x - w / 2.0, y + gap),
            (x + gap, y - h / 2.0),
            (x - w - gap, y - h / 2.0),
            (x - w / 2.0, y - h / 2.0),
            (x + gap, y - h - gap),
            (x - w - gap, y - h - gap),
            (x + gap, y + gap),
            (x - w - gap, y + gap),
        ];
        if let Some((origin, rect)) = origins.into_iter().find_map(|origin| {
            let rect = Rect {
                left: origin.0,
                top: origin.1,
                right: origin.0 + w,
                bottom: origin.1 + h,
            };
            (inside(rect, bounds) && placed.iter().all(|other| !overlaps(rect, *other, gap)))
                .then_some((origin, rect))
        }) {
            placed.push(rect);
            output[index] = Placement {
                x: origin.0,
                y: origin.1,
                visible: true,
            };
        } else {
            output[index] = Placement {
                x: x - w / 2.0,
                y: y - h / 2.0,
                visible: false,
            };
        }
    }
    output
}

fn inside(rect: Rect, bounds: [f64; 4]) -> bool {
    rect.left >= bounds[0]
        && rect.top >= bounds[1]
        && rect.right <= bounds[2]
        && rect.bottom <= bounds[3]
}

fn overlaps(a: Rect, b: Rect, padding: f64) -> bool {
    !(a.right + padding <= b.left
        || b.right + padding <= a.left
        || a.bottom + padding <= b.top
        || b.bottom + padding <= a.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_wins_and_ties_keep_input_order() {
        let candidates = [
            Candidate {
                point: [50.0, 50.0],
                size: [40.0, 20.0],
                priority: 1.0,
            },
            Candidate {
                point: [50.0, 50.0],
                size: [40.0, 20.0],
                priority: 3.0,
            },
        ];
        let placed = place(&candidates, [0.0, 0.0, 100.0, 100.0], 4.0);
        assert!(placed[1].visible);
        assert!(
            placed[0].visible,
            "a second legal side should still be found"
        );
        assert_ne!((placed[0].x, placed[0].y), (placed[1].x, placed[1].y));
    }

    #[test]
    fn impossible_label_is_hidden_not_moved_outside() {
        let placed = place(
            &[Candidate {
                point: [5.0, 5.0],
                size: [200.0, 100.0],
                priority: 0.0,
            }],
            [0.0, 0.0, 100.0, 50.0],
            2.0,
        );
        assert!(!placed[0].visible);
    }
}
