//! Picking the up face from gravity, and carrying north along when the cube
//! rolls onto another face.

use crate::geom::{dot, nearest_axis, Axis, Heading};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpConfig {
    /// The cube must tilt this far off its current up face before another
    /// face can take over. Above 45° so a deep walking tilt doesn't roll the
    /// map.
    pub switch_deg: f32,
    /// ...and stay past it this many frames (a roll in progress).
    pub settle_frames: u8,
}

impl Default for UpConfig {
    fn default() -> Self {
        UpConfig {
            switch_deg: 55.0,
            settle_frames: 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UpTracker {
    heading: Heading,
    pending: Option<(Axis, u8)>,
    rolls: u32,
}

impl UpTracker {
    pub fn new(heading: Heading) -> Self {
        UpTracker {
            heading,
            pending: None,
            rolls: 0,
        }
    }

    pub fn heading(&self) -> Heading {
        self.heading
    }

    /// How many times the up face has changed.
    pub fn rolls(&self) -> u32 {
        self.rolls
    }

    /// Feed the sky direction in die axes (any length; the accelerometer at
    /// rest). Returns true when the up face changed.
    pub fn update(&mut self, up: [f32; 3], cfg: &UpConfig) -> bool {
        let len = libm::sqrtf(up[0] * up[0] + up[1] * up[1] + up[2] * up[2]);
        if len < 1e-3 {
            return false;
        }
        let cur = self.heading.up;
        let along = (up[0] * cur[0] as f32 + up[1] * cur[1] as f32 + up[2] * cur[2] as f32) / len;
        let best = nearest_axis(up);
        if best == cur || along > libm::cosf(cfg.switch_deg.to_radians()) {
            self.pending = None;
            return false;
        }
        let count = match self.pending {
            Some((a, n)) if a == best => n.saturating_add(1),
            _ => 1,
        };
        if count < cfg.settle_frames.max(1) {
            self.pending = Some((best, count));
            return false;
        }
        self.pending = None;
        // A face two rolls away (upside down) still goes through
        // `rolled_to`, which turns about east.
        debug_assert!(dot(best, cur) <= 0);
        self.heading = self.heading.rolled_to(best);
        self.rolls += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Compass;

    #[test]
    fn small_tilts_keep_the_face_and_a_settled_roll_changes_it() {
        let cfg = UpConfig::default();
        let mut t = UpTracker::default();
        let h0 = t.heading();
        // 40° toward north: walking, not rolling.
        let a = 40f32.to_radians();
        let n = h0.north;
        let tilt = |a: f32| [0, 1, 2].map(|k| libm::cosf(a) * h0.up[k] as f32 + libm::sinf(a) * n[k] as f32);
        for _ in 0..30 {
            assert!(!t.update(tilt(a), &cfg));
        }
        // Tipped over the north edge, the south face comes up: in die axes
        // the sky swings toward the south face.
        let s = h0.dir(Compass::South);
        let over = [0, 1, 2].map(|k| 0.2 * h0.up[k] as f32 + s[k] as f32);
        let mut changed = false;
        for _ in 0..cfg.settle_frames {
            changed |= t.update(over, &cfg);
        }
        assert!(changed);
        assert_eq!(t.heading().up, s);
        assert_eq!(t.heading().north, h0.up);
    }
}
