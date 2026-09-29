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
//!
//! One turn can pass several faces: turning the die 180° in one go counts as
//! two tips, and turning back part of the way takes steps off again. The turn
//! ends when the die comes to rest on a face, and counts the faces it moved.
//! Whatever a turn leaves over (the die rarely stops exactly square) carries
//! into the next one, so the count follows the die's real orientation even
//! through slow turns with stops on the way.
//!
//! Real hands aren't rails. Stillness is judged on a smoothed rate, so hand
//! tremor averages out. A turn mostly about the line of sight (a twist, a
//! regrip) is handling, not a tip. And whenever the die comes to rest the
//! firmware squares its frame to gravity ([`Frame::resync`]), which catches
//! tilts, rolls and drift; only turns about vertical (left and right tips)
//! rely on the gyro alone.
//!
//! The thresholds below are first guesses from the simulator and need
//! tuning against recordings from the real IMU.

use libm::{fabsf, roundf, sqrtf};
use smokebomb_hal::Face;

/// Below this the die counts as still (deg/s, smoothed rate).
const STILL_DPS: f32 = 8.0;
/// Stillness and starts are judged on the rate averaged over this many
/// samples (100 ms at 60 Hz): about one period of hand tremor (7–11 Hz),
/// which mostly cancels out, and short enough that a turn that has ended is
/// forgotten quickly.
const RATE_WINDOW: usize = 6;
/// Above this a turn has started (deg/s). Low, so a slow turn counts too.
const START_DPS: f32 = 12.0;
/// Stillness needed before tips are recognised (after the menu opens, or
/// after the die is snapped toward the viewer).
const ARM_MS: u64 = 120;
/// Angle at which the direction is decided.
const DECIDE_DEG: f32 = 8.0;
/// A turn that stops within this many quarter turns of a face ends there
/// (30°: a quick tip that stops at 60° counts, as before).
const SETTLE: f32 = 1.0 / 3.0;
/// A turn ends once the die has been still this long, so a hand (or a
/// pointer) that stops briefly mid-turn doesn't end it early.
const SETTLE_STILL_MS: u64 = 150;
/// A turn that stops between faces for this long ends at the nearest face.
const SETTLE_MS: u64 = 1_000;
/// Leftovers smaller than this (degrees) when a turn settles on a face are
/// sensor error, not a real offset, and are dropped so they can't build up
/// over many tips.
const NOISE_DEG: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipDir {
    Up,
    Down,
    Left,
    Right,
}

impl TipDir {
    pub const fn opposite(self) -> TipDir {
        match self {
            TipDir::Up => TipDir::Down,
            TipDir::Down => TipDir::Up,
            TipDir::Left => TipDir::Right,
            TipDir::Right => TipDir::Left,
        }
    }
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
    /// itself points up or down (looking down at it in a palm, or up at it
    /// overhead), gravity gives no "up" (SIM_SPEC H7); `fallback_up` (the way
    /// the screen already reads, any length) stands in for it.
    pub fn new(front: Face, up: [f32; 3], fallback_up: [f32; 3]) -> Frame {
        let f = normal(front);
        let mut u = snap(up);
        if fabsf(dot(u, f)) > 0.5 {
            u = snap(fallback_up);
            if fabsf(dot(u, f)) > 0.5 {
                u = [f[1], f[2], f[0]]; // any axis perpendicular to f
            }
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

    /// Square the frame to gravity `up` (any length), measured with the die
    /// at rest. Returns the new frame, the up/down tips the die has made
    /// that the gyro missed (a slow tilt onto the next face counts, as a tip
    /// would), and whether it was rolled about the line of sight (the sky
    /// moved to a side; no step).
    pub fn resync(&self, up: [f32; 3]) -> (Frame, i32, bool) {
        let sky = snap(up);
        if sky == self.up {
            return (*self, 0, false);
        }
        if fabsf(dot(sky, self.right)) < 0.5 {
            // Tilted about the viewer's right: the tips that would do that.
            for k in [1, -1, 2] {
                let f = self.stepped(TipDir::Up, k);
                if f.up == sky {
                    return (f, k, false);
                }
            }
        }
        // Rolled: same face in front, the sky now off to one side.
        (
            Frame {
                front: self.front,
                up: sky,
                right: cross(sky, self.front),
            },
            0,
            true,
        )
    }

    /// The frame after `steps` tips in `dir` (negative steps go the other way).
    pub fn stepped(&self, dir: TipDir, steps: i32) -> Frame {
        let d = if steps < 0 { dir.opposite() } else { dir };
        let mut f = *self;
        for _ in 0..steps.unsigned_abs() {
            f = f.after(d);
        }
        f
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
    /// A turn is under way: its direction, and how far it has turned in
    /// quarter turns. Past 1 it has passed a face; below 0 it has come back
    /// past where it started.
    Turning {
        dir: TipDir,
        progress: f32,
    },
    /// A turn came to rest `steps` faces along `dir` (negative: the other
    /// way).
    Done {
        dir: TipDir,
        steps: i32,
    },
    /// The die moved but came back to the face it started on.
    Cancelled,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TipTracker {
    armed: bool,
    still_since: Option<u64>,
    turning: bool,
    /// Rotation since the last face was counted, including any leftover
    /// (degrees, die axes).
    theta: [f32; 3],
    /// Rotation since this turn started; picks the turn's direction.
    fresh: [f32; 3],
    dir: Option<TipDir>,
    /// When the die stopped mid-turn.
    paused_since: Option<u64>,
    /// What the last turn left over, about the frame's right and up axes
    /// (degrees). Those are the viewer's right and the sky, which stay put
    /// as the frame turns, so the leftover stays meaningful.
    leftover: (f32, f32),
    /// Rotation (degrees, die axes) that started the next turn on the tick
    /// the last one was counted.
    carry: Option<[f32; 3]>,
    /// The last few angular rates (deg/s, die axes), for the average.
    rates: [[f32; 3]; RATE_WINDOW],
    next_rate: usize,
    /// Rotation since the die was last still, while no turn is under way:
    /// the start of a turn, before the smoothed rate shows it.
    since_still: [f32; 3],
    /// This turn is mostly about the line of sight: handling, not a tip.
    handling: bool,
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

    /// End the turn `steps` faces along `dir`, keeping what's left over.
    fn settle(&mut self, dir: TipDir, steps: f32, frame: &Frame) -> TipUpdate {
        let counted = frame.axis(dir).map(|a| a * steps * 90.0);
        let mut left: [f32; 3] = core::array::from_fn(|i| self.theta[i] - counted[i]);
        if steps != 0.0 {
            // Only the overshoot along the tip's own axis carries over. Its
            // wobble off that axis happened in a frame the tip has since
            // turned, so carried as right/up it would land on the wrong axis
            // and pile up over tips; gravity squares what matters of it.
            let axis = frame.axis(dir);
            let along = dot(left, axis);
            left = axis.map(|a| a * along);
        }
        self.leftover = if norm(left) < NOISE_DEG {
            (0.0, 0.0)
        } else {
            (dot(left, frame.right), dot(left, frame.up))
        };
        self.turning = false;
        self.theta = [0.0; 3];
        self.dir = None;
        self.handling = false;
        self.paused_since = None;
        match steps as i32 {
            0 => TipUpdate::Cancelled,
            steps => TipUpdate::Done { dir, steps },
        }
    }

    /// Gravity has squared the frame up (see [`Frame::resync`]): leftover
    /// tilt about the viewer's right is accounted for, and after a roll the
    /// old leftovers mean nothing. Leftover turn about vertical stays, as
    /// gravity can't see it.
    pub fn resynced(&mut self, rolled: bool) {
        self.leftover = if rolled {
            (0.0, 0.0)
        } else {
            (0.0, self.leftover.1)
        };
    }

    /// Signed progress in quarter turns along `dir`'s axis.
    fn progress(&self, dir: TipDir, frame: &Frame) -> f32 {
        dot(self.theta, frame.axis(dir)) / 90.0
    }

    /// Feed one gyro sample (milli-deg/s, die axes) covering `dt_s` seconds.
    /// Returns `(armed_now, update)`: `armed_now` is true on the tick the
    /// tracker becomes armed, when the caller should rebuild its frame.
    pub fn update(&mut self, gyro_mdps: [i32; 3], dt_s: f32, now: u64, frame: &Frame) -> (bool, TipUpdate) {
        let w = gyro_mdps.map(|g| g as f32 / 1000.0);
        self.rates[self.next_rate] = w;
        self.next_rate = (self.next_rate + 1) % RATE_WINDOW;
        let mean: [f32; 3] =
            core::array::from_fn(|i| self.rates.iter().map(|r| r[i]).sum::<f32>() / RATE_WINDOW as f32);
        let speed = norm(mean);
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
            let carry = self.carry.take();
            if still {
                self.since_still = [0.0; 3];
            } else {
                for (a, w) in self.since_still.iter_mut().zip(w) {
                    *a += w * dt_s;
                }
            }
            if carry.is_none() && speed < START_DPS {
                return (false, TipUpdate::None);
            }
            self.turning = true;
            // Everything since the die was last still: the smoothed rate
            // takes a few samples to show a turn has begun.
            let (r, u) = self.leftover;
            let start = match carry {
                Some(c) => c,
                None => core::mem::take(&mut self.since_still),
            };
            self.theta = core::array::from_fn(|i| r * frame.right[i] + u * frame.up[i] + start[i]);
            self.fresh = start;
            self.dir = None;
            self.handling = false;
            // This sample is already in `since_still`.
            if carry.is_none() {
                return (false, TipUpdate::None);
            }
        }

        // Moving again after resting on a face: that turn is over, even if it
        // rested less than the usual settle time, and this is a new one. A
        // quick second tip, perhaps in another direction, would otherwise be
        // folded into the first and never counted.
        if !still && self.paused_since.is_some() {
            if let Some(dir) = self.dir {
                let p = self.progress(dir, frame);
                let steps = roundf(p);
                if fabsf(p - steps) <= SETTLE {
                    let result = self.settle(dir, steps, frame);
                    self.carry = Some(w.map(|c| c * dt_s));
                    return (false, result);
                }
            }
        }

        for ((t, f), w) in self.theta.iter_mut().zip(&mut self.fresh).zip(w) {
            *t += w * dt_s;
            *f += w * dt_s;
        }
        if self.dir.is_none() && !self.handling && norm(self.fresh) >= DECIDE_DEG {
            let (r, u) = (dot(self.fresh, frame.right), dot(self.fresh, frame.up));
            let twist = dot(self.fresh, frame.front);
            // Mostly about the line of sight: a twist or a regrip.
            self.handling = fabsf(twist) > fabsf(r).max(fabsf(u));
        }
        if self.dir.is_none() && !self.handling && norm(self.fresh) >= DECIDE_DEG {
            let (r, u) = (dot(self.fresh, frame.right), dot(self.fresh, frame.up));
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
            let paused = now.saturating_sub(*self.paused_since.get_or_insert(now));
            if paused >= SETTLE_STILL_MS {
                match self.dir {
                    None => return (false, self.settle(TipDir::Up, 0.0, frame)),
                    Some(dir) => {
                        let p = self.progress(dir, frame);
                        let steps = roundf(p);
                        if fabsf(p - steps) <= SETTLE || paused >= SETTLE_MS {
                            return (false, self.settle(dir, steps, frame));
                        }
                        // Stopped between faces: wait.
                    }
                }
            }
        } else {
            self.paused_since = None;
        }
        match self.dir {
            Some(dir) => {
                // At rest near a face, show it there while the turn settles,
                // rather than a few degrees off and then snapping.
                let p = self.progress(dir, frame);
                let near = roundf(p);
                let progress = if still && fabsf(p - near) <= SETTLE {
                    near
                } else {
                    p
                };
                (false, TipUpdate::Turning { dir, progress })
            }
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
        Frame::new(Face::PosZ, [0.0, 1000.0, 0.0], [0.0, 1.0, 0.0])
    }

    /// Gravity for a die whose +Z face leans back `deg` from vertical (toward
    /// the sky), +Y still the upper edge: the up vector in die axes.
    fn leaning(deg: f32) -> [f32; 3] {
        let r = deg.to_radians();
        [0.0, 1000.0 * libm::cosf(r), 1000.0 * libm::sinf(r)]
    }

    #[test]
    fn a_leaning_hold_keeps_the_upright_frame_up_to_45_degrees() {
        for deg in [0.0, 20.0, 40.0] {
            let f = Frame::new(Face::PosZ, leaning(deg), [1.0, 0.0, 0.0]);
            assert_eq!(f.up, [0.0, 1.0, 0.0], "{deg}");
            assert_eq!(f.right, [1.0, 0.0, 0.0], "{deg}");
        }
    }

    #[test]
    fn past_45_degrees_the_frame_follows_the_screen_not_an_arbitrary_axis() {
        // Screen mostly facing the sky (60° lean): gravity now snaps to the
        // front axis, so up comes from the fallback, which is the way the
        // screen already reads (+Y here).
        let f = Frame::new(Face::PosZ, leaning(60.0), [0.0, 1.0, 0.0]);
        assert_eq!(f.up, [0.0, 1.0, 0.0]);
        assert_eq!(f.right, [1.0, 0.0, 0.0]);
        // Looking up at it overhead (face pointing at the floor): same rule.
        let f = Frame::new(Face::PosZ, [0.0, 0.0, -1000.0], [1.0, 0.0, 0.0]);
        assert_eq!(f.up, [1.0, 0.0, 0.0]);
        assert_eq!(f.next_front(TipDir::Down), Face::PosX);
    }

    #[test]
    fn an_unusable_fallback_still_gives_a_valid_frame() {
        let f = Frame::new(Face::PosZ, [0.0, 0.0, 1000.0], [0.0, 0.0, 1.0]);
        assert_eq!(dot(f.up, f.front), 0.0);
        assert_eq!(dot(f.right, f.front), 0.0);
        assert_eq!(dot(f.right, f.up), 0.0);
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
        turn_by(t, f, axis, 90.0)
    }

    fn turn_by(t: &mut TipTracker, f: &Frame, axis: [f32; 3], degrees: f32) -> std::vec::Vec<TipUpdate> {
        let mut out = std::vec::Vec::new();
        let dt = 1.0 / 60.0;
        let mut now = 1_000u64;
        let mut prev = 0.0;
        for i in 1..=40 {
            let u = ((i as f32 * dt) / 0.42).min(1.0);
            let angle = degrees * (1.0 - (1.0 - u).powi(3));
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
            assert!(
                updates.contains(&TipUpdate::Done { dir, steps: 1 }),
                "{dir:?}: {updates:?}"
            );
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
    fn a_long_turn_counts_every_face_it_passes() {
        let f = frame();
        let mut t = armed();
        let updates = turn_by(&mut t, &f, f.axis(TipDir::Left), 270.0);
        assert!(updates.contains(&TipUpdate::Done {
            dir: TipDir::Left,
            steps: 3
        }));
        let max = updates
            .iter()
            .filter_map(|u| match u {
                TipUpdate::Turning { progress, .. } => Some(*progress),
                _ => None,
            })
            .fold(0.0, f32::max);
        assert!(max > 2.9, "{max}");
        assert_eq!(f.stepped(TipDir::Left, 3), f.after(TipDir::Right));
        assert_eq!(f.stepped(TipDir::Left, -1), f.after(TipDir::Right));
    }

    #[test]
    fn stopping_between_faces_waits_then_settles_on_the_nearest() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        // 135° about the Left axis over 0.5 s, then still.
        let g = f.axis(TipDir::Left).map(|a| (a * 270.0 * 1000.0) as i32);
        let mut now = 300;
        for _ in 0..30 {
            now += 17;
            t.update(g, dt, now, &f);
        }
        // Still, a sample at a time: it waits between faces for a while...
        let still = |t: &mut TipTracker, now: &mut u64, n: usize| {
            (0..n)
                .map(|_| {
                    *now += 17;
                    t.update([0; 3], dt, *now, &f).1
                })
                .collect::<std::vec::Vec<_>>()
        };
        let waiting = still(&mut t, &mut now, 30);
        assert!(
            waiting.iter().all(|u| matches!(u, TipUpdate::Turning { .. })),
            "waits between faces: {waiting:?}"
        );
        // ...then settles on the nearest face.
        let later = still(&mut t, &mut now, 40);
        assert!(
            later.contains(&TipUpdate::Done {
                dir: TipDir::Left,
                steps: 2
            }),
            "{later:?}"
        );
    }

    /// Three 40° turns with long stops: each alone is nearer no face than the
    /// start, but together they are a face and a bit.
    #[test]
    fn leftovers_carry_into_the_next_turn() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        let axis = f.axis(TipDir::Left);
        let mut now = 300;
        let mut all = std::vec::Vec::new();
        for _ in 0..3 {
            for _ in 0..10 {
                now += 17;
                all.push(t.update(axis.map(|a| (a * 240.0 * 1000.0) as i32), dt, now, &f).1);
            }
            for _ in 0..90 {
                now += 17;
                all.push(t.update([0; 3], dt, now, &f).1);
            }
        }
        let steps: i32 = all
            .iter()
            .map(|u| match u {
                TipUpdate::Done { steps, .. } => *steps,
                _ => 0,
            })
            .sum();
        assert_eq!(steps, 1, "{all:?}");
    }

    /// A gyro that reads 4% high (sensor scale error) still counts one step
    /// per tip over a long run, and each tip rests exactly on its face.
    #[test]
    fn scale_error_does_not_build_up() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        let mut now = 300;
        let mut steps = 0;
        let mut frame = f;
        for _ in 0..13 {
            let mut prev = 0.0;
            let mut last_turning = None;
            for i in 1..=45 {
                let u = ((i as f32 * dt) / 0.42).min(1.0);
                let angle = 90.0 * (1.0 - (1.0 - u).powi(3));
                let rate = (angle - prev) / dt * 1.04;
                prev = angle;
                now += 17;
                let axis = frame.axis(TipDir::Left);
                match t
                    .update(axis.map(|a| (a * rate * 1000.0) as i32), dt, now, &frame)
                    .1
                {
                    TipUpdate::Turning { progress, .. } => last_turning = Some(progress),
                    TipUpdate::Done { steps: n, .. } => {
                        steps += n;
                        frame = frame.stepped(TipDir::Left, n);
                    }
                    _ => {}
                }
            }
            assert_eq!(last_turning, Some(1.0), "rests on the face, centred");
        }
        assert_eq!(steps, 13);
    }

    #[test]
    fn a_nudge_is_not_a_tip() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        // 60°/s for 0.2 s = 12°, about the vertical (a left tip's axis).
        let g = f.axis(TipDir::Left).map(|a| (a * 60_000.0) as i32);
        let mut last = TipUpdate::None;
        for i in 0..12 {
            last = t.update(g, dt, 300 + i * 17, &f).1;
        }
        assert!(matches!(last, TipUpdate::Turning { .. }), "{last:?}");
        let after: std::vec::Vec<_> = (1..30)
            .map(|i| t.update([0; 3], dt, 504 + i * 17, &f).1)
            .collect();
        assert!(after.contains(&TipUpdate::Cancelled), "{after:?}");
        assert!(!after.iter().any(|u| matches!(u, TipUpdate::Done { .. })));
    }

    #[test]
    fn a_twist_is_handling_not_a_tip() {
        let f = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        // 90° about the line of sight (+Z, toward the viewer).
        let g = [0, 0, 270_000];
        let mut seen = std::vec::Vec::new();
        for i in 0..20 {
            seen.push(t.update(g, dt, 300 + i * 17, &f).1);
        }
        for i in 0..30 {
            seen.push(t.update([0; 3], dt, 640 + i * 17, &f).1);
        }
        assert!(
            !seen
                .iter()
                .any(|u| matches!(u, TipUpdate::Turning { .. } | TipUpdate::Done { .. })),
            "{seen:?}"
        );
        assert!(seen.contains(&TipUpdate::Cancelled));
    }

    #[test]
    fn resync_squares_the_frame_to_gravity() {
        let f = frame(); // +Z front, +Y up
                         // No change.
        assert_eq!(f.resync([0.0, 980.0, 30.0]), (f, 0, false));
        // Tilted forward onto the next face: that's an up tip.
        let (g, steps, rolled) = f.resync([0.0, 0.0, 1000.0]);
        assert_eq!((g, steps, rolled), (f.after(TipDir::Up), 1, false));
        // Rolled a quarter turn: same face in front, the sky to the side.
        let (g, steps, rolled) = f.resync([1000.0, 0.0, 0.0]);
        assert_eq!(
            (g.front, g.up, steps, rolled),
            (f.front, [1.0, 0.0, 0.0], 0, true)
        );
        assert_eq!(g.right, cross(g.up, g.front));
    }

    /// A turn made in small nudges with short stops (a slow hand, or a
    /// browser sending sparse pointer moves) still counts every face.
    #[test]
    fn a_turn_in_nudges_counts_every_face() {
        let mut frame = frame();
        let mut t = armed();
        let dt = 1.0 / 60.0;
        let mut now = 300;
        let mut steps = 0;
        let mut feed = |t: &mut TipTracker, frame: &mut Frame, now: u64, g: [i32; 3]| {
            if let TipUpdate::Done { dir, steps: n } = t.update(g, dt, now, frame).1 {
                assert_eq!(dir, TipDir::Up);
                steps += n;
                *frame = frame.stepped(dir, n);
            }
        };
        // 20 nudges of 13.5° (270° in all), each followed by 150 ms still. A
        // stop near a face ends that part of the turn, so it may be counted
        // in pieces; together they must come to three faces.
        for _ in 0..20 {
            for _ in 0..3 {
                now += 17;
                let g = frame.axis(TipDir::Up).map(|a| (a * 270.0 * 1000.0) as i32); // 4.5° per frame
                feed(&mut t, &mut frame, now, g);
            }
            for _ in 0..9 {
                now += 17;
                feed(&mut t, &mut frame, now, [0; 3]);
            }
        }
        for _ in 0..15 {
            now += 17;
            feed(&mut t, &mut frame, now, [0; 3]);
        }
        assert_eq!(steps, 3);
    }
}
