//! Scene camera mapping from World to Screen after layout and location resolution. Apply it to the
//! entire scene rather than individual layers. Following an unavailable target preserves the camera
//! state to avoid jumps.

use crate::geom::Point;
use crate::program::recording::{Affine, ProgramRecording, RecordCmd};
use crate::space::{Mapping, Screen, SpaceRect, World};

/// Evaluated camera position, zoom, and rotation for one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// World-space point mapped to the viewport center.
    pub center: Point,
    /// Positive zoom factor; 1 preserves scale and larger values zoom in. Invalid values fall back
    /// to 1.
    pub zoom: f64,
    /// Clockwise rotation in degrees, matching Affine and CSS rotation.
    pub rotation_deg: f64,
}

impl Camera {
    /// Create an identity camera centered on the viewport.
    pub fn still(viewport: (f64, f64)) -> Camera {
        Camera {
            center: Point::new(viewport.0 / 2.0, viewport.1 / 2.0),
            zoom: 1.0,
            rotation_deg: 0.0,
        }
    }

    /// Translate the camera center to the origin, scale, rotate, then translate to the viewport
    /// center.
    pub fn to_screen(&self, viewport: (f64, f64)) -> Mapping<World, Screen> {
        let zoom = if self.zoom.is_finite() && self.zoom > 0.0 {
            self.zoom
        } else {
            1.0
        };
        let m = Affine::translate(-self.center.x, -self.center.y)
            .then(Affine::scale(zoom, zoom))
            .then(Affine::rotate(self.rotation_deg))
            .then(Affine::translate(viewport.0 / 2.0, viewport.1 / 2.0));
        Mapping::new(m)
    }

    /// Center on a located target without changing zoom or rotation. Preserve the camera when no
    /// target is available.
    pub fn follow(self, target: Option<SpaceRect<World>>) -> Camera {
        match target {
            None => self,
            Some(rect) => Camera {
                center: rect.center().point,
                ..self
            },
        }
    }

    /// Fit the target plus a relative margin into the viewport using the smaller axis scale. Ignore
    /// degenerate axes; preserve zoom when both are degenerate.
    pub fn zoom_to(self, target: SpaceRect<World>, viewport: (f64, f64), margin: f64) -> Camera {
        let grow = 1.0 + margin.max(0.0);
        let (w, h) = (target.rect.width * grow, target.rect.height * grow);
        let fx = (w > 0.0).then(|| viewport.0 / w);
        let fy = (h > 0.0).then(|| viewport.1 / h);
        let zoom = match (fx, fy) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => self.zoom,
        };
        Camera {
            center: target.center().point,
            zoom: if zoom.is_finite() && zoom > 0.0 {
                zoom
            } else {
                self.zoom
            },
            ..self
        }
    }
}

/// Wrap the entire recording in an outer camera transform, preserving concatenated inner
/// transforms. Identity cameras add no commands.
pub fn apply(list: &mut ProgramRecording, camera: Mapping<World, Screen>) {
    let m = camera.affine();
    if m == Affine::IDENTITY {
        return;
    }
    list.cmds
        .insert(0, RecordCmd::BeginTransform { transform: m });
    list.push(RecordCmd::End);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Rect;
    use crate::space::SpacePoint;

    const VP: (f64, f64) = (400.0, 200.0);

    fn world(x: f64, y: f64) -> SpacePoint<World> {
        SpacePoint::xy(x, y)
    }

    /// An identity camera leaves both coordinates and commands unchanged.
    #[test]
    fn a_still_camera_changes_nothing() {
        let m = Camera::still(VP).to_screen(VP);
        assert_eq!(m.affine(), Affine::IDENTITY);
        let p = m.point(world(37.0, 91.0));
        assert_eq!(p.point, Point::new(37.0, 91.0));

        let mut list = ProgramRecording::new();
        list.push(RecordCmd::BeginGroup);
        list.push(RecordCmd::End);
        let before = list.cmds.len();
        apply(&mut list, m);
        assert_eq!(
            list.cmds.len(),
            before,
            "identity camera must not emit empty groups"
        );
    }

    /// Zoom keeps the camera target fixed at the viewport center.
    #[test]
    fn zoom_expands_around_the_camera_center() {
        let cam = Camera {
            center: Point::new(100.0, 50.0),
            zoom: 2.0,
            rotation_deg: 0.0,
        };
        let m = cam.to_screen(VP);
        // Map the camera center to the viewport center.
        assert_eq!(m.point(world(100.0, 50.0)).point, Point::new(200.0, 100.0));
        // Double the point's offset from the camera center.
        assert_eq!(m.point(world(110.0, 50.0)).point, Point::new(220.0, 100.0));
    }

    #[test]
    fn rotation_turns_the_scene_around_the_center() {
        let cam = Camera {
            center: Point::new(0.0, 0.0),
            zoom: 1.0,
            rotation_deg: 90.0,
        };
        let p = cam.to_screen(VP).point(world(10.0, 0.0)).point;
        // Rotate clockwise by 90 degrees before centering in the viewport.
        assert!(
            (p.x - 200.0).abs() < 1e-9 && (p.y - 110.0).abs() < 1e-9,
            "{p:?}"
        );
    }

    /// Following changes the center without affecting zoom or rotation.
    #[test]
    fn following_a_target_moves_only_the_center() {
        let cam = Camera {
            center: Point::new(0.0, 0.0),
            zoom: 3.0,
            rotation_deg: 15.0,
        };
        let target = SpaceRect::<World>::new(Rect::new(50.0, 20.0, 40.0, 10.0));
        let followed = cam.follow(Some(target));
        assert_eq!(followed.center, Point::new(70.0, 25.0));
        assert_eq!(followed.zoom, 3.0, "following must preserve zoom");
        assert_eq!(followed.rotation_deg, 15.0);
    }

    /// Preserve the camera when the target is unavailable.
    #[test]
    fn a_missing_target_leaves_the_camera_untouched() {
        let cam = Camera {
            center: Point::new(12.0, 34.0),
            zoom: 2.5,
            rotation_deg: 7.0,
        };
        assert_eq!(cam.follow(None), cam);
    }

    /// Fit the target and margin using the smaller axis scale.
    #[test]
    fn zooming_to_a_target_fits_it_within_the_margin() {
        let cam = Camera::still(VP);
        // The horizontal axis limits the fit to 10x.
        let target = SpaceRect::<World>::new(Rect::new(0.0, 0.0, 40.0, 10.0));
        let z = cam.zoom_to(target, VP, 0.0);
        assert_eq!(z.zoom, 10.0);
        assert_eq!(z.center, Point::new(20.0, 5.0));
        // A relative margin reduces the fit scale.
        let z = cam.zoom_to(target, VP, 0.1);
        assert!((z.zoom - 400.0 / 44.0).abs() < 1e-9, "{}", z.zoom);
    }

    /// Degenerate targets preserve zoom.
    #[test]
    fn a_degenerate_target_leaves_zoom_alone() {
        let cam = Camera {
            center: Point::new(0.0, 0.0),
            zoom: 2.0,
            rotation_deg: 0.0,
        };
        let dot = SpaceRect::<World>::new(Rect::new(5.0, 5.0, 0.0, 0.0));
        let z = cam.zoom_to(dot, VP, 0.0);
        assert_eq!(z.zoom, 2.0);
        assert_eq!(
            z.center,
            Point::new(5.0, 5.0),
            "the center must remain aligned"
        );
    }

    /// Invalid zoom falls back to 1.
    #[test]
    fn a_bad_zoom_degrades_to_one_instead_of_erasing_the_frame() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let cam = Camera {
                center: Point::new(200.0, 100.0),
                zoom: bad,
                rotation_deg: 0.0,
            };
            assert_eq!(cam.to_screen(VP).affine(), Affine::IDENTITY, "zoom = {bad}");
        }
    }

    /// Wrapping a recording preserves balanced transform groups.
    #[test]
    fn applying_a_camera_wraps_the_whole_list() {
        let mut list = ProgramRecording::new();
        let path = list
            .begin_path()
            .rect(Rect::new(0.0, 0.0, 10.0, 10.0))
            .finish();
        list.push(RecordCmd::Path {
            path,
            fill_rule: crate::program::recording::FillRule::NonZero,
            fill: Some(crate::program::recording::Paint::Solid(
                crate::color::Rgba::rgb(255, 0, 0),
            )),
            stroke: None,
        });
        let cam = Camera {
            center: Point::new(0.0, 0.0),
            zoom: 2.0,
            rotation_deg: 0.0,
        };
        apply(&mut list, cam.to_screen(VP));

        assert!(
            matches!(list.cmds.first(), Some(RecordCmd::BeginTransform { .. })),
            "the camera must wrap the outermost layer"
        );
        assert!(matches!(list.cmds.last(), Some(RecordCmd::End)));
        assert_eq!(
            list.validate(),
            Ok(()),
            "wrapping must preserve structural validity"
        );
    }
}
