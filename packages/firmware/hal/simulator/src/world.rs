//! The physical die: where it is, how it's oriented, and what its IMU reads.
//!
//! The simulator server owns the die's pose. Browser gestures (shake, throw,
//! tip, turn, dock) become motions here; each tick produces the pose the UI
//! draws and the accelerometer/gyro sample the firmware reads. The UI never
//! moves the die on its own, so what the firmware senses always matches what
//! you see.
//!
//! Orientation and angular rate come straight from the rendered motion, which
//! follows the mockup's animation curves (SIM_SPEC C3, C5). Linear
//! acceleration is synthesized per phase so the firmware sees a physically
//! plausible hand shake, free fall and impacts: the mockup's stylised bounce
//! (a 21 mm hop over 1.25 s) implies almost no acceleration by itself.
//!
//! It lives in the HAL crate so the firmware's tests can drive the same
//! motions the simulator does.

use glam::{Quat, Vec3};
use smokebomb_hal::{Face, ImuSample};

const G_MG: f32 = 1000.0;
/// Specific force during a single-tick impact, on top of gravity.
const IMPACT_MG: f32 = 3000.0;
/// Below this height (scene units) the die counts as touching the table.
const AIRBORNE_ABOVE: f32 = 0.05;
/// The firmware needs a few shake samples before a release reads as a throw.
const MIN_SHAKE_S: f64 = 0.05;
/// The simulator's camera sits along (0.55, 0.62, 1) looking at the die
/// (SIM_SPEC A5); this is its right, (1, 0, −0.55) normalised.
pub const DEFAULT_VIEWER_RIGHT: Vec3 = Vec3::new(0.876_356, 0.0, -0.481_996);
/// How long the menu takes to turn the held face toward the viewer.
const SNAP_S: f64 = 0.35;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Die body → world rotation (three.js `die.quaternion`).
    pub rotation: Quat,
    /// Die centre in scene units, relative to resting on the table.
    pub position: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipDir {
    Up,
    Down,
    Left,
    Right,
}

struct Tumble {
    start: f64,
    duration: f64,
    from: Quat,
    to: Quat,
    spin_axis: Vec3,
    /// Which bounce segment (0.4 u each) the last tick was in.
    segment: u32,
}

struct Turn {
    start: f64,
    duration: f64,
    from: Quat,
    to: Quat,
    hop: bool,
}

pub struct World {
    pose: Pose,
    time: f64,
    shake_start: Option<f64>,
    throw_when_ready: bool,
    tumble: Option<Tumble>,
    turn: Option<Turn>,
    /// The viewer's right in world space (horizontal). Tips use it as the
    /// up/down axis; the menu snap turns the held face toward the viewer.
    viewer_right: Vec3,
    docked: bool,
    reduced_motion: bool,
    rng: u64,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        let mut seed = [0u8; 8];
        let _ = getrandom::getrandom(&mut seed);
        Self {
            // Mockup load orientation: 0.35 rad about vertical, +Y up.
            pose: Pose {
                rotation: Quat::from_rotation_y(0.35),
                position: Vec3::ZERO,
            },
            time: 0.0,
            shake_start: None,
            throw_when_ready: false,
            tumble: None,
            turn: None,
            viewer_right: DEFAULT_VIEWER_RIGHT,
            docked: false,
            reduced_motion: false,
            rng: u64::from_le_bytes(seed) | 1,
        }
    }

    pub fn pose(&self) -> Pose {
        self.pose
    }

    pub fn docked(&self) -> bool {
        self.docked
    }

    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
    }

    /// Mid-throw: other motions are ignored until the die lands.
    pub fn tumbling(&self) -> bool {
        self.tumble.is_some()
    }

    /// Start shaking the die in the hand (the mockup's throw button, held).
    pub fn start_shake(&mut self) {
        if self.tumble.is_some() {
            return;
        }
        self.docked = false;
        self.turn = None;
        self.shake_start.get_or_insert(self.time);
    }

    /// Stop shaking; `throw` releases the die, otherwise it's put down.
    pub fn end_shake(&mut self, throw: bool) {
        if self.shake_start.is_none() {
            return;
        }
        if throw {
            self.throw_when_ready = true;
        } else {
            self.shake_start = None;
            self.pose.position.y = 0.0;
        }
    }

    /// Tip the die a quarter turn. `right` is the viewer's right in world
    /// space (the axis for up/down tips); left/right tips turn about vertical.
    /// Returns false if the die is busy.
    pub fn tip(&mut self, dir: TipDir, right: Vec3) -> bool {
        if self.busy() {
            return false;
        }
        let (axis, angle) = match dir {
            TipDir::Up => (right, -std::f32::consts::FRAC_PI_2),
            TipDir::Down => (right, std::f32::consts::FRAC_PI_2),
            TipDir::Left => (Vec3::Y, -std::f32::consts::FRAC_PI_2),
            TipDir::Right => (Vec3::Y, std::f32::consts::FRAC_PI_2),
        };
        let axis = axis.try_normalize().unwrap_or(Vec3::X);
        if matches!(dir, TipDir::Up | TipDir::Down) {
            self.viewer_right = axis;
        }
        let to = Quat::from_axis_angle(axis, angle) * self.pose.rotation;
        let duration = if self.reduced_motion { 0.15 } else { 0.42 };
        self.turn = Some(Turn {
            start: self.time,
            duration,
            from: self.pose.rotation,
            to,
            hop: true,
        });
        true
    }

    /// Turn the die so `face` looks at the viewer, squared up to the view
    /// (the mockup's `alignedQuat` plus its fix-up turn when the menu opens).
    /// On the real die the person does this; the simulator does it for them.
    pub fn snap_to_viewer(&mut self, face: Face) {
        if self.busy() || self.docked {
            return;
        }
        let r = self.viewer_right;
        let front = r.cross(Vec3::Y);
        let basis = Quat::from_mat3(&glam::Mat3::from_cols(r, Vec3::Y, front));
        let local = glam::Mat3::from_quat(basis.inverse() * self.pose.rotation);
        let sx = snap_axis(local.x_axis);
        let mut sy = snap_axis(local.y_axis);
        if sx.dot(sy).abs() > 0.5 {
            sy = snap_axis(local.z_axis.cross(local.x_axis));
        }
        let snapped = Quat::from_mat3(&glam::Mat3::from_cols(sx, sy, sx.cross(sy)));
        let aligned = basis * snapped;
        let n = aligned * face_normal(face);
        let half = std::f32::consts::FRAC_PI_2;
        let fix = if n.dot(front) >= 0.9 {
            Quat::IDENTITY
        } else if n.y.abs() > 0.9 {
            Quat::from_axis_angle(r, if n.y > 0.0 { half } else { -half })
        } else if n.dot(r) > 0.9 {
            Quat::from_rotation_y(-half)
        } else if n.dot(r) < -0.9 {
            Quat::from_rotation_y(half)
        } else {
            Quat::from_rotation_y(std::f32::consts::PI)
        };
        self.turn = Some(Turn {
            start: self.time,
            duration: SNAP_S,
            from: self.pose.rotation,
            to: (fix * aligned).normalize(),
            hop: false,
        });
    }

    /// Turn the die in the hand by world-axis angles (the mockup's drag:
    /// yaw about vertical, then pitch about the world X axis).
    pub fn rotate(&mut self, yaw: f32, pitch: f32) {
        if self.busy() || self.docked {
            return;
        }
        self.pose.rotation =
            (Quat::from_rotation_x(pitch) * Quat::from_rotation_y(yaw) * self.pose.rotation).normalize();
    }

    /// Set the die down with `face` on top, taking the shortest turn.
    pub fn place_face_up(&mut self, face: Face) {
        if self.busy() || self.docked {
            return;
        }
        let normal = self.pose.rotation * face_normal(face);
        let to = Quat::from_rotation_arc(normal.normalize(), Vec3::Y) * self.pose.rotation;
        self.turn = Some(Turn {
            start: self.time,
            duration: 0.35,
            from: self.pose.rotation,
            to,
            hop: false,
        });
    }

    pub fn set_docked(&mut self, docked: bool) {
        if docked {
            self.shake_start = None;
            self.throw_when_ready = false;
            self.tumble = None;
            self.turn = None;
        }
        self.docked = docked;
    }

    fn busy(&self) -> bool {
        self.tumble.is_some() || self.turn.is_some() || self.shake_start.is_some()
    }

    /// Advance by `dt` seconds; returns what the IMU reads afterwards.
    pub fn step(&mut self, dt: f64) -> ImuSample {
        self.time += dt;
        let before = self.pose.rotation;
        let ease_back = 1.0 - (-6.0 * dt as f32).exp();
        // Linear acceleration in world space, milli-g (gravity excluded).
        let mut linear = Vec3::ZERO;

        if let Some(start) = self.shake_start {
            let t = (self.time - start) as f32;
            if !self.reduced_motion {
                let yaw = self.rand_signed() * 0.06;
                let pitch = self.rand_signed() * 0.06;
                self.pose.rotation =
                    (Quat::from_rotation_x(pitch) * Quat::from_rotation_y(yaw) * self.pose.rotation)
                        .normalize();
                self.pose.position.x += self.rand_signed() * 0.03;
                self.pose.position.y = (self.time as f32 * 23.0).sin().abs() * 0.08;
            }
            // A vigorous hand shake. The vertical part stays small so the
            // total never dips toward zero g (which would read as a throw).
            let w = std::f32::consts::TAU;
            linear = Vec3::new(
                1300.0 * (w * 6.0 * t).sin(),
                400.0 * (w * 5.0 * t).sin(),
                900.0 * (w * 7.0 * t).cos(),
            );
            if self.throw_when_ready && self.time - start >= MIN_SHAKE_S {
                self.begin_tumble();
            }
        } else if self.tumble.is_none() {
            self.pose.position.x -= self.pose.position.x * ease_back;
        }

        if let Some(tb) = &mut self.tumble {
            let u = ((self.time - tb.start) / tb.duration).min(1.0) as f32;
            let e = ease_out_cubic(u);
            let mut q = tb.from.slerp(tb.to, e);
            if !self.reduced_motion {
                q = Quat::from_axis_angle(tb.spin_axis, (1.0 - e) * 4.0 * std::f32::consts::PI) * q;
            }
            self.pose.rotation = q;
            // The mockup's bounce: zero height at u = 0.4 and 0.8.
            let height = (2.5 * std::f32::consts::PI * u).sin().abs() * (1.0 - u).powi(2) * 1.6;
            self.pose.position.y = if self.reduced_motion { 0.0 } else { height };
            self.pose.position.x -= self.pose.position.x * ease_back;
            let segment = (u / 0.4) as u32;
            if segment != tb.segment && u < 1.0 {
                linear = Vec3::Y * IMPACT_MG;
            } else if height > AIRBORNE_ABOVE {
                linear = -Vec3::Y * G_MG; // free fall: the sensor reads ~0 g
            }
            tb.segment = segment;
            if u >= 1.0 {
                self.pose.rotation = tb.to;
                self.pose.position = Vec3::ZERO;
                self.tumble = None;
            }
        }

        if let Some(turn) = &self.turn {
            let u = ((self.time - turn.start) / turn.duration).min(1.0) as f32;
            self.pose.rotation = turn.from.slerp(turn.to, ease_out_cubic(u));
            self.pose.position.y = if turn.hop {
                (std::f32::consts::PI * u).sin() * 0.12
            } else {
                0.0
            };
            if u >= 1.0 {
                self.pose.rotation = turn.to;
                self.pose.position.y = 0.0;
                self.turn = None;
            }
        }

        if self.docked {
            // Settles upright in the Nest.
            self.pose.rotation = self.pose.rotation.slerp(Quat::IDENTITY, ease_back);
            self.pose.position -= self.pose.position * ease_back;
        }

        self.imu(before, linear, dt as f32)
    }

    fn begin_tumble(&mut self) {
        self.shake_start = None;
        self.throw_when_ready = false;
        self.docked = false;
        let quarter = |r: &mut Self| (r.rand_unit() * 4.0).floor() * std::f32::consts::FRAC_PI_2;
        let (a, b, c) = (quarter(self), quarter(self), quarter(self));
        let yaw = Quat::from_rotation_y(self.rand_signed() * 0.35);
        let to = yaw * Quat::from_rotation_x(a) * Quat::from_rotation_y(b) * Quat::from_rotation_z(c);
        let spin_axis = Vec3::new(self.rand_signed(), self.rand_signed(), self.rand_signed())
            .try_normalize()
            .unwrap_or(Vec3::Y);
        self.tumble = Some(Tumble {
            start: self.time,
            duration: if self.reduced_motion { 0.45 } else { 1.25 },
            from: self.pose.rotation,
            to,
            spin_axis,
            segment: 0,
        });
    }

    fn imu(&self, before: Quat, linear_world_mg: Vec3, dt: f32) -> ImuSample {
        let inv = self.pose.rotation.inverse();
        // At rest the accelerometer reads +1 g toward the sky.
        let force = inv * (Vec3::Y * G_MG + linear_world_mg);
        let (axis, angle) = (self.pose.rotation * before.inverse()).to_axis_angle();
        let angle = if angle > std::f32::consts::PI {
            angle - std::f32::consts::TAU
        } else {
            angle
        };
        let omega_body = inv * (axis * (angle / dt.max(1e-6)));
        let mdps = omega_body * (180.0 / std::f32::consts::PI * 1000.0);
        ImuSample {
            accel_mg: [clamp_i16(force.x), clamp_i16(force.y), clamp_i16(force.z)],
            gyro_mdps: [mdps.x as i32, mdps.y as i32, mdps.z as i32],
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn rand_unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [-0.5, 0.5), like the mockup's `Math.random() - 0.5`.
    fn rand_signed(&mut self) -> f32 {
        self.rand_unit() - 0.5
    }
}

pub fn face_normal(face: Face) -> Vec3 {
    match face {
        Face::PosX => Vec3::X,
        Face::NegX => -Vec3::X,
        Face::PosY => Vec3::Y,
        Face::NegY => -Vec3::Y,
        Face::PosZ => Vec3::Z,
        Face::NegZ => -Vec3::Z,
    }
}

/// The unit axis nearest `v`, with its sign.
fn snap_axis(v: Vec3) -> Vec3 {
    let a = v.abs();
    let sign = |x: f32| if x < 0.0 { -1.0 } else { 1.0 };
    if a.x >= a.y && a.x >= a.z {
        Vec3::X * sign(v.x)
    } else if a.y >= a.z {
        Vec3::Y * sign(v.y)
    } else {
        Vec3::Z * sign(v.z)
    }
}

fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

fn clamp_i16(v: f32) -> i16 {
    v.clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 60.0;

    fn run(world: &mut World, seconds: f64) -> Vec<ImuSample> {
        (0..(seconds / DT).round() as usize)
            .map(|_| world.step(DT))
            .collect()
    }

    fn magnitude(s: &ImuSample) -> f32 {
        Vec3::new(s.accel_mg[0] as f32, s.accel_mg[1] as f32, s.accel_mg[2] as f32).length()
    }

    #[test]
    fn at_rest_reads_one_g_up_and_no_rotation() {
        let mut w = World::new();
        let s = w.step(DT);
        assert!((magnitude(&s) - 1000.0).abs() < 2.0);
        assert!(s.accel_mg[1] > 990, "+Y is up at load: {s:?}");
        assert!(s.gyro_mdps.iter().all(|g| g.abs() < 10));
    }

    #[test]
    fn throw_has_free_fall_and_impacts_then_rests_on_a_face() {
        let mut w = World::new();
        w.start_shake();
        let shake = run(&mut w, 0.5);
        assert!(
            shake.iter().any(|s| (magnitude(s) - 1000.0).abs() > 700.0),
            "shake is vigorous"
        );
        assert!(
            shake.iter().all(|s| magnitude(s) > 350.0),
            "shake never reads as free fall"
        );
        w.end_shake(true);
        let flight = run(&mut w, 1.4);
        assert!(flight.iter().any(|s| magnitude(s) < 100.0), "free fall");
        assert_eq!(
            flight.iter().filter(|s| magnitude(s) > 2500.0).count(),
            2,
            "two bounces"
        );
        assert!(!w.tumbling());
        let rest = w.step(DT);
        let a = rest.accel_mg.map(|v| v.unsigned_abs());
        assert!(a.iter().any(|&v| v > 990), "lands squarely on a face: {rest:?}");
    }

    #[test]
    fn tip_turns_a_quarter_with_matching_gyro() {
        let mut w = World::new();
        let before = w.pose().rotation;
        assert!(w.tip(TipDir::Left, Vec3::X));
        let samples = run(&mut w, 0.5);
        let turned = w.pose().rotation * before.inverse();
        let (axis, angle) = turned.to_axis_angle();
        assert!((angle - std::f32::consts::FRAC_PI_2).abs() < 1e-3);
        assert!(axis.y.abs() > 0.99);
        // Integrated gyro ≈ 90°.
        let deg: f32 = samples
            .iter()
            .map(|s| (s.gyro_mdps[1] as f32 / 1000.0) * DT as f32)
            .sum();
        assert!((deg.abs() - 90.0).abs() < 1.0, "{deg}");
    }

    #[test]
    fn place_face_up_turns_that_face_to_the_sky() {
        let mut w = World::new();
        w.place_face_up(Face::NegZ);
        run(&mut w, 0.5);
        let s = w.step(DT);
        assert!(s.accel_mg[2] < -990, "{s:?}");
    }

    #[test]
    fn snap_turns_the_held_face_to_the_viewer() {
        let front = DEFAULT_VIEWER_RIGHT.cross(Vec3::Y);
        for face in Face::ALL {
            let mut w = World::new();
            w.rotate(0.9, 0.3);
            w.snap_to_viewer(face);
            run(&mut w, 0.5);
            let q = w.pose().rotation;
            assert!((q * face_normal(face)).dot(front) > 0.999, "{face:?}");
            // Squared up: every die axis lies along a viewer axis.
            for n in [Vec3::X, Vec3::Y, Vec3::Z] {
                let v = q * n;
                assert!(
                    v.dot(front)
                        .abs()
                        .max(v.y.abs())
                        .max(v.dot(DEFAULT_VIEWER_RIGHT).abs())
                        > 0.999
                );
            }
        }
    }

    #[test]
    fn docking_settles_upright() {
        let mut w = World::new();
        w.set_docked(true);
        run(&mut w, 2.0);
        assert!(w.pose().rotation.angle_between(Quat::IDENTITY) < 0.01);
    }
}
