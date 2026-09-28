//! Recognising menu tips from the gyro (SIM_SPEC C3, "Tips").
//!
//! In the menu the die is held with one face toward the person (the front).
//! A tip is a quick quarter-turn that brings a neighbouring face to the front:
//!
//! | Tip   | Rotation (viewer's frame)      | Comes to the front |
//! |-------|--------------------------------|--------------------|
//! | up    | −90° about the viewer's right  | bottom face        |
//! | down  | +90° about the viewer's right  | top face           |
//! | left  | −90° about vertical            | right face         |
//! | right | +90° about vertical            | left face          |
//!
//! Left and right turn about the gravity axis, so the accelerometer can't see
//! them; the gyro can. The tracker integrates body-frame angular rate while
//! the die turns and classifies the accumulated rotation against the menu
//! [`Frame`]. Progress (angle turned ÷ 90°) drives the content slide, which in
//! the mockup follows the same ease-out curve as the rotation.

use libm::{fabsf, sqrtf};
use smokebomb_hal::Face;

/// Below this the die counts as still (deg/s).
const STILL_DPS: f32 = 8.0;
/// Above this a turn has started (deg/s).
const START_DPS: f32 = 30.0;
/// Stillness needed before tips are recognised (after the menu opens, or
/// after the die is snapped toward the viewer).
const ARM_MS: u64 = 120;
/// Angle at which the direction is decided, and the least that counts as a
/// completed tip when the turn stops.
const DECIDE_DEG: f32 = 8.0;
const COMPLETE_DEG: f32 = 60.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipDir {
    Up,
    Down,
    Left,
    Right,
}

/// The menu's frame in die coordinates: unit vectors toward the viewer
/// (front), the sky (up) and the viewer's right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub front: [f32; 3],
    pub up: [f32; 3],
    pub right: [f32; 3],
}

impl Frame {
    /// The frame for a held `front` face with gravity `up` (any length). The
    /// up vector is snapped to the face axis nearest the sky. If the held face
    /// itself points up or down, "up" is undefined (SIM_SPEC H7); the next
    /// face around is used so the frame stays valid.
    pub fn new(front: Face, up: [f32; 3]) -> Frame {
        let f = normal(front);
        let mut u = snap(up);
        if fabsf(dot(u, f)) > 0.5 {
            u = [f[1], f[2], f[0]]; // any axis perpendicular to f
        }
        Frame {
            front: f,
            up: u,
            right: cross(u, f),
        }
    }

    pub fn front_face(&self) -> Face {
        face_of(self.front)
    }

    /// The face that comes to the front after `dir`.
    pub fn next_front(&self, dir: TipDir) -> Face {
        face_of(match dir {
            TipDir::Up => neg(self.up),
            TipDir::Down => self.up,
            TipDir::Left => self.right,
            TipDir::Right => neg(self.right),
        })
    }

    /// The frame once `dir` has finished: the face that came to the front,
    /// with the sky wherever the turn left it.
    pub fn after(&self, dir: TipDir) -> Frame {
        let (front, up) = match dir {
            // The bottom comes to the front; the old front goes on top.
            TipDir::Up => (neg(self.up), self.front),
            TipDir::Down => (self.up, neg(self.front)),
            TipDir::Left => (self.right, self.up),
            TipDir::Right => (neg(self.right), self.up),
        };
        Frame {
            front,
            up,
            right: cross(up, front),
        }
    }

    /// Which way `face`'s surface moves during `dir`, as a unit vector in
    /// its unrotated canvas (x right, y down); zero for faces on the axis.
    /// The mockup slides menu content along it (`faceMotionDir`).
    pub fn motion_dir(&self, face: Face, dir: TipDir) -> (f32, f32) {
        let v = cross(self.axis(dir), normal(face));
        if dot(v, v) < 0.01 {
            return (0.0, 0.0);
        }
        let b = &crate::orientation::BASES[face.index()];
        let (x, y) = (dot(v, b.x), -dot(v, b.y));
        let len = sqrtf(x * x + y * y);
        if len > 0.0 {
            (x / len, y / len)
        } else {
            (0.0, 0.0)
        }
    }

    /// Rotation axis (with sign, as a unit vector in die coordinates) of `dir`.
    pub fn axis(&self, dir: TipDir) -> [f32; 3] {
        match dir {
            TipDir::Up => neg(self.right),
            TipDir::Down => self.right,
            TipDir::Left => neg(self.up),
            TipDir::Right => self.up,
        }
    }
}

/// What the tracker saw this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TipUpdate {
    None,
    /// A tip is under way: its direction and how far it has turned (0–1).
    Turning {
        dir: TipDir,
        progress: f32,
    },
    /// A tip finished.
    Done(TipDir),
    /// The die moved but didn't complete a tip.
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TipTracker {
    armed: bool,
    still_since: Option<u64>,
    turning: bool,
    theta: [f32; 3],
    dir: Option<TipDir>,
}

impl TipTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop recognising tips until the die has been still for a moment.
    pub fn disarm(&mut self) {
        *self = Self::default();
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn turning(&self) -> Option<(TipDir, f32)> {
        match (self.turning, self.dir) {
            (true, Some(d)) => Some((d, (norm(self.theta) / 90.0).min(1.0))),
            _ => None,
        }
    }

    /// Feed one gyro sample (milli-deg/s, die axes) covering `dt_s` seconds.
    /// Returns `(armed_now, update)`: `armed_now` is true on the tick the
    /// tracker becomes armed, when the caller should rebuild its frame.
    pub fn update(&mut self, gyro_mdps: [i32; 3], dt_s: f32, now: u64, frame: &Frame) -> (bool, TipUpdate) {
        let w = gyro_mdps.map(|g| g as f32 / 1000.0);
        let speed = norm(w);
        let still = speed < STILL_DPS;

        if !self.armed {
            if !still {
                self.still_since = None;
                return (false, TipUpdate::None);
            }
            let since = *self.still_since.get_or_insert(now);
            if now - since >= ARM_MS {
                self.armed = true;
                return (true, TipUpdate::None);
            }
            return (false, TipUpdate::None);
        }

        if !self.turning {
            if speed < START_DPS {
                return (false, TipUpdate::None);
            }
            self.turning = true;
            self.theta = [0.0; 3];
            self.dir = None;
        }

        for (t, w) in self.theta.iter_mut().zip(w) {
            *t += w * dt_s;
        }
        let angle = norm(self.theta);
        if self.dir.is_none() && angle >= DECIDE_DEG {
            let (r, u) = (dot(self.theta, frame.right), dot(self.theta, frame.up));
            self.dir = Some(if fabsf(r) >= fabsf(u) {
                if r < 0.0 {
                    TipDir::Up
                } else {
                    TipDir::Down
                }
            } else if u < 0.0 {
                TipDir::Left
            } else {
                TipDir::Right
            });
        }

        if still {
            let result = match self.dir {
                Some(d) if angle >= COMPLETE_DEG => TipUpdate::Done(d),
                _ => TipUpdate::Cancelled,
            };
            self.turning = false;
            self.theta = [0.0; 3];
            self.dir = None;
            return (false, result);
        }
        match self.dir {
            Some(dir) => (
                false,
                TipUpdate::Turning {
                    dir,
                    progress: (angle / 90.0).min(1.0),
                },
            ),
            None => (false, TipUpdate::None),
        }
    }
}

pub fn normal(face: Face) -> [f32; 3] {
    match face {
        Face::PosX => [1.0, 0.0, 0.0],
        Face::NegX => [-1.0, 0.0, 0.0],
        Face::PosY => [0.0, 1.0, 0.0],
        Face::NegY => [0.0, -1.0, 0.0],
        Face::PosZ => [0.0, 0.0, 1.0],
        Face::NegZ => [0.0, 0.0, -1.0],
    }
}

/// The face whose normal is closest to `v`.
pub fn face_of(v: [f32; 3]) -> Face {
    let a = v.map(fabsf);
    let axis = if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    };
    let positive = v[axis] >= 0.0;
    Face::ALL[axis * 2 + if positive { 0 } else { 1 }]
}

fn snap(v: [f32; 3]) -> [f32; 3] {
    normal(face_of(v))
}

pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn neg(v: [f32; 3]) -> [f32; 3] {
    [-v[0], -v[1], -v[2]]
}

fn norm(v: [f32; 3]) -> f32 {
    sqrtf(dot(v, v))
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    /// Upright die, +Z toward the viewer: right is +X.
    fn frame() -> Frame {
        Frame::new(Face::PosZ, [0.0, 1000.0, 0.0])
    }

    #[test]
    fn frame_axes() {
        let f = frame();
        assert_eq!(f.right, [1.0, 0.0, 0.0]);
        assert_eq!(f.next_front(TipDir::Up), Face::NegY);
        assert_eq!(f.next_front(TipDir::Down), Face::PosY);
        assert_eq!(f.next_front(TipDir::Left), Face::PosX);
        assert_eq!(f.next_front(TipDir::Right), Face::NegX);
        for dir in [TipDir::Up, TipDir::Down, TipDir::Left, TipDir::Right] {
            let after = f.after(dir);
            assert_eq!(after.front_face(), f.next_front(dir), "{dir:?}");
            assert_eq!(after.right, cross(after.up, after.front));
        }
        // Tipping up carries the bottom face up toward the viewer: its
        // surface moves toward its +Z edge, the top of its canvas, and new
        // content slides in from that leading edge.
        assert_eq!(f.after(TipDir::Up).up, f.front);
        assert_eq!(f.motion_dir(Face::NegY, TipDir::Up), (0.0, -1.0));
    }

    /// Simulate a 90° turn about `axis` over 0.42 s with the mockup's
    /// ease-out, then rest.
    fn turn(t: &mut TipTracker, f: &Frame, axis: [f32; 3]) -> std::vec::Vec<TipUpdate> {
        let mut out = std::vec::Vec::new();
        let dt = 1.0 / 60.0;
        let mut now = 1_000u64;
        let mut prev = 0.0;
        for i in 1..=40 {
            let u = ((i as f32 * dt) / 0.42).min(1.0);
            let angle = 90.0 * (1.0 - (1.0 - u).powi(3));
            let rate = (angle - prev) / dt;
            prev = angle;
            let g = axis.map(|a| (a * rate * 1000.0) as i32);
            now += 17;
            out.push(t.update(g, dt, now, f).1);
        }
        out
    }

    fn armed() -> TipTracker {
        let mut t = TipTracker::new();
        let f = frame();
        assert!(!t.update([0; 3], 0.016, 0, &f).0);
        assert!(t.update([0; 3], 0.016, 200, &f).0);
        t
    }

    #[test]
    fn recognises_each_direction() {
        let f = frame();
        for dir in [TipDir::Up, TipDir::Down, TipDir::Left, TipDir::Right] {
            let mut t = armed();
            let updates = turn(&mut t, &f, f.axis(dir));
            assert!(updates.contains(&TipUpdate::Done(dir)), "{dir:?}: {updates:?}");
            let max_progress = updates
                .iter()
                .filter_map(|u| match u {
                    TipUpdate::Turning { progress, .. } => Some(*progress),
                    _ => None,
                })
                .fold(0.0, f32::max);
            assert!(max_progress > 0.9, "{dir:?}: {max_progress}");
        }
    }

    #[test]
    fn a_nudge_is_not_a_tip() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        let g = [0, 0, 60_000]; // 60°/s for 0.2 s = 12°
        let mut last = TipUpdate::None;
        for i in 0..12 {
            last = t.update(g, dt, 300 + i * 17, &f).1;
        }
        assert!(matches!(last, TipUpdate::Turning { .. }));
        assert_eq!(t.update([0; 3], dt, 600, &f).1, TipUpdate::Cancelled);
    }
}
