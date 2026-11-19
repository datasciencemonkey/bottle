//! The MOG eye-tracking math and poll plan (lamp-alive).
//!
//! A MOG window is a small non-activating panel: it gets no mouse moves while the pointer is
//! outside it. So the host reads the global pointer position and this module decides what each MOG
//! sees: a look vector from the eye centre, clamped to the unit disc, with a dead zone near the
//! centre. The vector goes to the brain (event `mog:look`), which hands it to that MOG's view.
//!
//! The poll rate follows the distance: about 30 Hz while the pointer is within [`NEAR_PX`] of a
//! visible MOG window, about 4 Hz when it is further away or no MOG is visible.
//!
//! Privacy: only the look vector leaves this module. A pointer position is never logged, stored
//! or sent. Everything here is pure, so a test drives it with a fake [`CursorSource`].

use std::time::Duration;

/// The pointer is "near" a MOG within this distance (logical px) of its window rectangle.
pub const NEAR_PX: f64 = 400.0;
/// Poll period while the pointer is near a visible MOG (about 30 Hz).
pub const NEAR_PERIOD: Duration = Duration::from_millis(33);
/// Poll period while the pointer is far from every MOG, or no MOG is visible (about 4 Hz).
pub const FAR_PERIOD: Duration = Duration::from_millis(250);
/// Inside this distance (logical px) of the eye centre the vector is zero: the eyes look ahead.
pub const DEAD_ZONE_PX: f64 = 14.0;
/// At this distance (logical px) and beyond the vector has length 1.
pub const FULL_PX: f64 = 140.0;
/// The eye centre is this far below the top of the window (logical px), on the centre line.
pub const EYE_OFFSET_Y_PX: f64 = 72.0;
/// A new vector is sent only when an axis moved by this much (the view eases, so less is noise).
pub const SEND_STEP: f64 = 0.02;

/// A window rectangle in physical px, with its scale factor (physical px per logical px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowGeom {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub scale: f64,
}

/// A source of the global pointer position in physical px, desktop coordinates, top left origin.
/// `None` when it cannot be read. The app uses the Tauri cursor read (NSEvent `mouseLocation`
/// on macOS, `GetCursorPos` on Windows); a test uses a fake.
pub trait CursorSource {
    fn position(&mut self) -> Option<(f64, f64)>;
}

/// The look vector for a pointer at `(dx, dy)` logical px from the eye centre: the direction of
/// the offset, with a length that is 0 inside [`DEAD_ZONE_PX`], rises linearly to 1 at
/// [`FULL_PX`] and stays 1 beyond. y grows downward. Never longer than 1 and never NaN.
pub fn look_vector(dx: f64, dy: f64) -> (f64, f64) {
    if !dx.is_finite() || !dy.is_finite() {
        return (0.0, 0.0);
    }
    let dist = dx.hypot(dy);
    if dist <= DEAD_ZONE_PX {
        return (0.0, 0.0);
    }
    let len = ((dist - DEAD_ZONE_PX) / (FULL_PX - DEAD_ZONE_PX)).clamp(0.0, 1.0);
    (dx / dist * len, dy / dist * len)
}

/// Distance from a point to a rectangle (0 inside it). All in the same unit.
pub fn distance_to_rect(px: f64, py: f64, x: f64, y: f64, w: f64, h: f64) -> f64 {
    let dx = (x - px).max(0.0).max(px - (x + w));
    let dy = (y - py).max(0.0).max(py - (y + h));
    dx.hypot(dy)
}

/// The poll period for the distance (logical px) from the pointer to the nearest visible MOG.
/// `None`: no visible MOG, so poll slowly.
pub fn poll_period(nearest: Option<f64>) -> Duration {
    match nearest {
        Some(d) if d <= NEAR_PX => NEAR_PERIOD,
        _ => FAR_PERIOD,
    }
}

/// One look vector for one MOG.
#[derive(Debug, Clone, PartialEq)]
pub struct LookUpdate {
    pub mog_id: String,
    pub dx: f64,
    pub dy: f64,
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Per-MOG send state, so a still pointer sends nothing and a far pointer sends one reset.
#[derive(Debug, Default)]
pub struct LookPlanner {
    last: std::collections::BTreeMap<String, (f64, f64)>,
}

impl LookPlanner {
    /// Read the pointer once and plan: the updates to send, and the wait before the next read.
    /// `windows` are the visible MOG windows. A MOG that is gone loses its state.
    pub fn poll(
        &mut self,
        source: &mut dyn CursorSource,
        windows: &[(String, WindowGeom)],
    ) -> (Vec<LookUpdate>, Duration) {
        self.last
            .retain(|id, _| windows.iter().any(|(wid, _)| wid == id));
        let Some((cx, cy)) = source.position() else {
            return (Vec::new(), FAR_PERIOD);
        };
        let mut nearest: Option<f64> = None;
        let mut out = Vec::new();
        for (id, g) in windows {
            let s = if g.scale > 0.0 { g.scale } else { 1.0 };
            let dist = distance_to_rect(cx, cy, g.x, g.y, g.w, g.h) / s;
            nearest = Some(nearest.map_or(dist, |n: f64| n.min(dist)));
            let target = if dist <= NEAR_PX {
                let ex = g.x + g.w / 2.0;
                let ey = g.y + EYE_OFFSET_Y_PX * s;
                let (vx, vy) = look_vector((cx - ex) / s, (cy - ey) / s);
                (round2(vx), round2(vy))
            } else {
                (0.0, 0.0)
            };
            let prev = self.last.get(id).copied().unwrap_or((0.0, 0.0));
            let first = !self.last.contains_key(id);
            let to_zero = target == (0.0, 0.0) && prev != (0.0, 0.0);
            let moved =
                (target.0 - prev.0).abs() >= SEND_STEP || (target.1 - prev.1).abs() >= SEND_STEP;
            if to_zero || moved {
                self.last.insert(id.clone(), target);
                out.push(LookUpdate {
                    mog_id: id.clone(),
                    dx: target.0,
                    dy: target.1,
                });
            } else if first {
                self.last.insert(id.clone(), prev);
            }
        }
        (out, poll_period(nearest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Vec<Option<(f64, f64)>>);
    impl CursorSource for Fake {
        fn position(&mut self) -> Option<(f64, f64)> {
            if self.0.is_empty() {
                None
            } else {
                self.0.remove(0)
            }
        }
    }

    fn geom(x: f64) -> WindowGeom {
        WindowGeom {
            x,
            y: 100.0,
            w: 160.0,
            h: 184.0,
            scale: 1.0,
        }
    }

    #[test]
    fn the_dead_zone_is_zero_and_the_edge_of_it_starts_to_move() {
        assert_eq!(look_vector(0.0, 0.0), (0.0, 0.0));
        assert_eq!(look_vector(DEAD_ZONE_PX, 0.0), (0.0, 0.0));
        assert_eq!(look_vector(6.0, -8.0), (0.0, 0.0)); // 10 px, inside
        let (x, _) = look_vector(DEAD_ZONE_PX + 1.0, 0.0);
        assert!(x > 0.0 && x < 0.05, "{x}");
    }

    #[test]
    fn the_vector_is_clamped_to_the_unit_disc_and_points_at_the_cursor() {
        assert_eq!(look_vector(FULL_PX, 0.0), (1.0, 0.0));
        assert_eq!(look_vector(-5000.0, 0.0), (-1.0, 0.0));
        let (x, y) = look_vector(3000.0, 4000.0);
        assert!((x - 0.6).abs() < 1e-9 && (y - 0.8).abs() < 1e-9, "{x} {y}");
        let (x, y) = look_vector(0.0, 77.0);
        assert_eq!(x, 0.0);
        assert!(y > 0.0 && y < 1.0);
        assert!(look_vector(f64::NAN, 3.0) == (0.0, 0.0));
        assert!(look_vector(f64::INFINITY, 3.0) == (0.0, 0.0));
    }

    #[test]
    fn distance_to_a_rectangle_is_zero_inside_and_euclidean_at_a_corner() {
        assert_eq!(distance_to_rect(50.0, 50.0, 0.0, 0.0, 100.0, 100.0), 0.0);
        assert_eq!(distance_to_rect(130.0, 50.0, 0.0, 0.0, 100.0, 100.0), 30.0);
        assert_eq!(distance_to_rect(-30.0, -40.0, 0.0, 0.0, 100.0, 100.0), 50.0);
    }

    #[test]
    fn the_poll_period_is_fast_near_and_slow_far_or_with_no_mog() {
        assert_eq!(poll_period(Some(0.0)), NEAR_PERIOD);
        assert_eq!(poll_period(Some(NEAR_PX)), NEAR_PERIOD);
        assert_eq!(poll_period(Some(NEAR_PX + 1.0)), FAR_PERIOD);
        assert_eq!(poll_period(None), FAR_PERIOD);
        assert!(NEAR_PERIOD.as_millis() <= 34 && FAR_PERIOD.as_millis() >= 250);
    }

    #[test]
    fn the_planner_backs_off_when_far_and_speeds_up_when_near() {
        let mut p = LookPlanner::default();
        let wins = vec![("a".to_owned(), geom(100.0))];
        let mut src = Fake(vec![
            Some((2000.0, 100.0)),
            Some((600.0, 190.0)),
            Some((900.0, 190.0)),
        ]);
        let (_, far) = p.poll(&mut src, &wins);
        assert_eq!(far, FAR_PERIOD);
        let (u, near) = p.poll(&mut src, &wins); // 340 px from the right edge: near
        assert_eq!(near, NEAR_PERIOD);
        assert_eq!(u.len(), 1);
        let (_, far2) = p.poll(&mut src, &wins); // 640 px: far again
        assert_eq!(far2, FAR_PERIOD);
    }

    #[test]
    fn the_planner_sends_on_change_only_and_resets_once_when_the_pointer_leaves() {
        let mut p = LookPlanner::default();
        let wins = vec![("a".to_owned(), geom(100.0))];
        let mut src = Fake(vec![
            Some((600.0, 172.0)),  // right of the eyes
            Some((600.0, 172.0)),  // same: nothing
            Some((601.0, 172.0)),  // 1 px: below the step
            Some((5000.0, 172.0)), // far: one reset
            Some((5001.0, 172.0)), // still far: nothing
        ]);
        let (u, _) = p.poll(&mut src, &wins);
        assert_eq!(
            u,
            vec![LookUpdate {
                mog_id: "a".into(),
                dx: 1.0,
                dy: 0.0
            }]
        );
        assert!(p.poll(&mut src, &wins).0.is_empty());
        assert!(p.poll(&mut src, &wins).0.is_empty());
        let (u, _) = p.poll(&mut src, &wins);
        assert_eq!(
            u,
            vec![LookUpdate {
                mog_id: "a".into(),
                dx: 0.0,
                dy: 0.0
            }]
        );
        assert!(p.poll(&mut src, &wins).0.is_empty());
    }

    #[test]
    fn the_planner_sends_nothing_for_a_pointer_in_the_dead_zone_it_never_left() {
        let mut p = LookPlanner::default();
        let wins = vec![("a".to_owned(), geom(100.0))];
        // The eye centre of this window is (180, 172).
        let mut src = Fake(vec![Some((185.0, 175.0))]);
        assert!(p.poll(&mut src, &wins).0.is_empty());
    }

    #[test]
    fn the_planner_uses_the_scale_factor_and_each_mog_has_its_own_vector() {
        let mut p = LookPlanner::default();
        let hi = WindowGeom {
            x: 0.0,
            y: 0.0,
            w: 320.0,
            h: 368.0,
            scale: 2.0,
        };
        let wins = vec![("a".to_owned(), hi), ("b".to_owned(), geom(1000.0))];
        // Eye centre of `a` is (160, 144) physical. The pointer is 300 px below it: 150 logical.
        let mut src = Fake(vec![Some((160.0, 444.0))]);
        let (u, _) = p.poll(&mut src, &wins);
        let a = u.iter().find(|x| x.mog_id == "a").unwrap();
        assert_eq!((a.dx, a.dy), (0.0, 1.0));
        // `b` is 840 px away (far): it had no vector, so it sends nothing.
        assert!(u.iter().all(|x| x.mog_id != "b"));
    }

    #[test]
    fn an_unreadable_pointer_or_no_window_is_slow_and_silent() {
        let mut p = LookPlanner::default();
        let wins = vec![("a".to_owned(), geom(0.0))];
        let (u, t) = p.poll(&mut Fake(vec![None]), &wins);
        assert!(u.is_empty());
        assert_eq!(t, FAR_PERIOD);
        let (u, t) = p.poll(&mut Fake(vec![Some((1.0, 1.0))]), &[]);
        assert!(u.is_empty());
        assert_eq!(t, FAR_PERIOD);
    }
}
