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
use smokebomb_hal::{Face, ImuSample, CHARGING_FACE};

const G_MG: f32 = 1000.0;
/// Specific force during a single-tick impact, on top of gravity.
const IMPACT_MG: f32 = 3000.0;
/// Below this height (scene units) the die counts as touching the table.
const AIRBORNE_ABOVE: f32 = 0.05;
/// The firmware needs a few shake samples before a release reads as a throw.
const MIN_SHAKE_S: f64 = 0.05;
/// Scene units to millimetres (SIM_SPEC A1: 1 u = 13.25 mm).
const MM_PER_U: f32 = 13.25;
/// The Nest's magnet is this far below the die's centre when it is seated
/// (the pocket floor, and the magnets under it), and gives this field there.
const NEST_MAGNET_DEPTH_MM: f32 = 20.0;
const NEST_FIELD_MG: f32 = 12_000.0;
/// The Earth's field in world axes (+Y up), milligauss: about 0.5 G, dipping.
const EARTH_MG: Vec3 = Vec3::new(200.0, -420.0, 100.0);
/// A stray magnet beside the die (a phone, a speaker, a fridge magnet): 7 G
/// at the die, pointing sideways at it. Strong enough to pass the field
/// strength test alone, so the direction check is what rejects it.
const STRAY_MG: Vec3 = Vec3::new(7_000.0, 0.0, 0.0);
/// How high the hand lifts the die out of the Nest (scene units): far enough
/// that the Nest's field is gone.
const LIFT_HEIGHT: f32 = 3.0;
/// The simulator's camera sits along (0.55, 0.62, 1) looking at the die
/// (SIM_SPEC A5); this is its right, (1, 0, −0.55) normalised.
pub const DEFAULT_VIEWER_RIGHT: Vec3 = Vec3::new(0.876_356, 0.0, -0.481_996);
/// How long the menu takes to turn the held face toward the viewer.
const SNAP_S: f64 = 0.35;
/// The pause after that turn before the die takes a tip (the firmware arms
/// its tip tracker after 120 ms still).
const SNAP_HOLD_S: f64 = 0.15;

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

/// Which axis a free spin turns about, as the menu's tips do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpinAxis {
    /// About vertical: left and right tips.
    Yaw,
    /// About the viewer's right: up and down tips.
    Pitch,
}

/// A spin that follows the pointer (the simulator's multi-turn gesture).
struct Spin {
    from: Quat,
    axis: Vec3,
    angle: f32,
    target: f32,
}

/// How a real hand moves the die, beyond the mockup's clean motions. All off
/// by default; tests turn them on to check the firmware copes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hand {
    /// Tremor while held: angular rate amplitude (deg/s) on each world axis,
    /// at 7–11 Hz.
    pub tremor_dps: f32,
    /// A constant gyro bias (deg/s, die axes): a sensor error, not motion.
    pub gyro_bias_dps: [f32; 3],
    /// A tip's axis is off by up to this (deg), about a random direction.
    pub tip_axis_error_deg: f32,
    /// A tip turns 90° ± up to this (deg).
    pub tip_angle_error_deg: f32,
    /// After a tip, the person squares the die up to look at it (0.3 s).
    pub resquare: bool,
}

struct Turn {
    start: f64,
    duration: f64,
    from: Quat,
    to: Quat,
    hop: bool,
    /// Held still this long after the motion before the die takes another
    /// (s).
    hold: f64,
    /// Square the die up to the viewer when this turn ends (a sloppy tip).
    resquare: bool,
}

pub struct World {
    pose: Pose,
    time: f64,
    shake_start: Option<f64>,
    throw_when_ready: bool,
    tumble: Option<Tumble>,
    turn: Option<Turn>,
    spin: Option<Spin>,
    /// The viewer's right in world space (horizontal). Tips use it as the
    /// up/down axis; the menu snap turns the held face toward the viewer.
    viewer_right: Vec3,
    /// Seated in the Nest, in `dock_rot`.
    docked: bool,
    /// The rotation it is seated in: the face down and a quarter-turn about
    /// vertical.
    dock_rot: Quat,
    /// A Nest is under the die's resting place (so its magnet is felt).
    nest_under: bool,
    /// A stray magnet sits beside the die.
    stray_magnet: bool,
    /// Held above the Nest (scene units), after a lift.
    hover: Option<f32>,
    /// The next step reads a small bump: the die was just set down.
    bump: bool,
    reduced_motion: bool,
    rng: u64,
    /// Tests: the face the next throw lands on, instead of a random one.
    next_landing: Option<Face>,
    hand: Hand,
    /// Tremor phases (rad), so the three axes don't move in step.
    tremor_phase: [f32; 3],
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
        Self::with_seed(u64::from_le_bytes(seed))
    }

    /// A world whose shakes and tumbles are the same every run (tests).
    pub fn with_seed(seed: u64) -> Self {
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
            spin: None,
            viewer_right: DEFAULT_VIEWER_RIGHT,
            docked: false,
            dock_rot: Quat::IDENTITY,
            nest_under: false,
            stray_magnet: false,
            hover: None,
            bump: false,
            reduced_motion: false,
            rng: seed | 1,
            next_landing: None,
            hand: Hand::default(),
            tremor_phase: [0.0, 2.1, 4.2],
        }
    }

    pub fn pose(&self) -> Pose {
        self.pose
    }

    pub fn docked(&self) -> bool {
        self.docked
    }

    /// The die is in the Nest with its charging face down, so the pogo
    /// pins reach the screws. Any of the face's four rotations works.
    pub fn on_charger(&self) -> bool {
        self.docked && (self.pose.rotation * face_normal(CHARGING_FACE)).y < -0.9
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
        self.spin = None;
        self.docked = false;
        self.nest_under = false;
        self.hover = None;
        self.turn = None;
        self.shake_start.get_or_insert(self.time);
    }

    /// Move like a real hand (tests).
    pub fn set_hand(&mut self, hand: Hand) {
        self.hand = hand;
    }

    /// Twist the die about the viewer's line of sight (a roll, not a tip)
    /// by `degrees`, clockwise as the viewer sees it. Returns false if busy.
    pub fn twist(&mut self, degrees: f32) -> bool {
        if self.busy() || self.docked {
            return false;
        }
        let front = self.viewer_right.cross(Vec3::Y);
        let to = Quat::from_axis_angle(front, -degrees.to_radians()) * self.pose.rotation;
        self.turn = Some(Turn {
            start: self.time,
            duration: 0.3,
            from: self.pose.rotation,
            to: to.normalize(),
            hop: false,
            hold: 0.0,
            resquare: false,
        });
        true
    }

    /// Make the next throw land with `up` on top (tests replaying a mockup
    /// throw that landed a known way).
    pub fn set_next_landing(&mut self, up: Face) {
        self.next_landing = Some(up);
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
        self.hover = None;
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
        // A real hand: the axis tilts a little and the angle is rarely 90°.
        let (axis, angle) = if self.hand.tip_axis_error_deg > 0.0 || self.hand.tip_angle_error_deg > 0.0 {
            let tilt = self.rand_signed() * 2.0 * self.hand.tip_axis_error_deg.to_radians();
            let spin = Quat::from_axis_angle(axis, self.rand_unit() * std::f32::consts::TAU);
            let tilted = Quat::from_axis_angle(spin * axis.any_orthonormal_vector(), tilt) * axis;
            let scale = 1.0 + self.rand_signed() * 2.0 * self.hand.tip_angle_error_deg / 90.0;
            (tilted, angle * scale)
        } else {
            (axis, angle)
        };
        let to = Quat::from_axis_angle(axis, angle) * self.pose.rotation;
        let duration = if self.reduced_motion { 0.15 } else { 0.42 };
        self.turn = Some(Turn {
            start: self.time,
            duration,
            from: self.pose.rotation,
            to,
            hop: true,
            hold: 0.0,
            resquare: self.hand.resquare,
        });
        true
    }

    /// Spin the die about a tip axis by `angle` radians from where the spin
    /// began, following the pointer: as many faces as you like, in one
    /// gesture. Signs match [`World::tip`]: positive yaw is a right tip,
    /// negative pitch an up tip. Returns false if the die is busy with
    /// something else.
    pub fn spin(&mut self, axis: SpinAxis, angle: f32, right: Vec3) -> bool {
        if self.spin.is_none() {
            if self.busy() || self.docked {
                return false;
            }
            let axis = match axis {
                SpinAxis::Yaw => Vec3::Y,
                SpinAxis::Pitch => {
                    let r = right.try_normalize().unwrap_or(self.viewer_right);
                    self.viewer_right = r;
                    r
                }
            };
            self.spin = Some(Spin {
                from: self.pose.rotation,
                axis,
                angle: 0.0,
                target: 0.0,
            });
        }
        if let Some(s) = &mut self.spin {
            s.target = angle;
        }
        true
    }

    /// Let go of a spin: the die settles on the nearest face.
    pub fn end_spin(&mut self) {
        let Some(s) = self.spin.take() else {
            return;
        };
        let quarter = std::f32::consts::FRAC_PI_2;
        let settled = (s.target / quarter).round() * quarter;
        let to = (Quat::from_axis_angle(s.axis, settled) * s.from).normalize();
        self.turn = Some(Turn {
            start: self.time,
            duration: if self.reduced_motion { 0.12 } else { 0.25 },
            from: self.pose.rotation,
            to,
            hop: false,
            hold: 0.0,
            resquare: false,
        });
    }

    /// Turn the die so `face` looks at the viewer, squared up to the view
    /// (the mockup's `alignedQuat` plus its fix-up turn when the menu opens).
    /// On the real die the person does this; the simulator does it for them.
    pub fn snap_to_viewer(&mut self, face: Face) {
        if self.busy() || self.docked {
            return;
        }
        self.turn = Some(Turn {
            start: self.time,
            duration: SNAP_S,
            from: self.pose.rotation,
            to: self.squared_toward_viewer(face),
            hop: false,
            // Having turned the die to face you, you pause before tipping
            // it; the firmware waits for that stillness to take its frame.
            hold: SNAP_HOLD_S,
            resquare: false,
        });
    }

    /// The die squared up to the view with `face` toward the viewer.
    fn squared_toward_viewer(&self, face: Face) -> Quat {
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
        (fix * aligned).normalize()
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
        self.hover = None;
        self.nest_under = false;
        let normal = self.pose.rotation * face_normal(face);
        let to = Quat::from_rotation_arc(normal.normalize(), Vec3::Y) * self.pose.rotation;
        self.turn = Some(Turn {
            start: self.time,
            duration: 0.35,
            from: self.pose.rotation,
            to,
            hop: false,
            hold: 0.0,
            resquare: false,
        });
    }

    /// Seat the die in the Nest the way it is sitting, squared up.
    pub fn set_docked(&mut self, docked: bool) {
        if docked {
            self.seat(self.squared());
        } else {
            self.lift();
        }
    }

    /// Put the die in the Nest with `down` toward the pocket floor, turned
    /// `quarters` quarter-turns about vertical. A Nest appears under it.
    /// The die settles into place over about a second.
    pub fn place_in_nest(&mut self, down: Face, quarters: u8) {
        self.seat(
            (Quat::from_rotation_y(quarters as f32 * std::f32::consts::FRAC_PI_2)
                * Quat::from_rotation_arc(face_normal(down), -Vec3::Y))
            .normalize(),
        );
    }

    /// Seat the die in the Nest, settling into `rot`.
    fn seat(&mut self, rot: Quat) {
        self.spin = None;
        self.shake_start = None;
        self.throw_when_ready = false;
        self.tumble = None;
        self.turn = None;
        self.hover = None;
        self.dock_rot = rot;
        self.docked = true;
        self.nest_under = true;
        self.bump = true;
        // Set down from a little above, so it is seen to drop in.
        self.pose.position.y = self.pose.position.y.max(0.25);
    }

    /// Lift the die out of the Nest and hold it in the hand above it. The
    /// Nest stays where it was; the die's next resting place is elsewhere.
    pub fn lift(&mut self) {
        if !self.docked {
            return;
        }
        self.docked = false;
        self.nest_under = false;
        self.hover = Some(LIFT_HEIGHT);
    }

    /// The face pointing down.
    pub fn face_down(&self) -> Face {
        Face::ALL
            .into_iter()
            .min_by(|a, b| {
                (self.pose.rotation * face_normal(*a))
                    .y
                    .total_cmp(&(self.pose.rotation * face_normal(*b)).y)
            })
            .unwrap_or(Face::NegY)
    }

    /// Put a stray magnet beside the die (or take it away).
    pub fn set_stray_magnet(&mut self, on: bool) {
        self.stray_magnet = on;
    }

    /// Whether the die sits in the Nest with power to be had: the die is
    /// seated, and settled into its pocket.
    pub fn seated(&self) -> bool {
        self.docked
            && self.pose.position.length() < 0.02
            && self.pose.rotation.angle_between(self.dock_rot) < 0.02
    }

    /// What the magnetometer's field is, in milligauss in the die's frame
    /// (before the die's own hard-iron offset): the Earth's, the Nest's
    /// magnet below (falling off with the cube of the distance as the die
    /// lifts), and a stray magnet if there is one.
    pub fn mag_field_mg(&self) -> [i32; 3] {
        let mut b = EARTH_MG;
        if self.nest_under {
            let d = NEST_MAGNET_DEPTH_MM + self.pose.position.y.max(0.0) * MM_PER_U;
            let k = (NEST_MAGNET_DEPTH_MM / d).powi(3);
            b += Vec3::NEG_Y * NEST_FIELD_MG * k;
        }
        if self.stray_magnet {
            b += STRAY_MG;
        }
        let die = self.pose.rotation.inverse() * b;
        [die.x as i32, die.y as i32, die.z as i32]
    }

    /// The current pose turned to the nearest face-aligned orientation, so
    /// whichever face is down stays down.
    fn squared(&self) -> Quat {
        let ex = snap_axis(self.pose.rotation * Vec3::X);
        let y = self.pose.rotation * Vec3::Y;
        let ey = snap_axis(y - ex * y.dot(ex));
        Quat::from_mat3(&glam::Mat3::from_cols(ex, ey, ex.cross(ey))).normalize()
    }

    fn busy(&self) -> bool {
        self.tumble.is_some() || self.turn.is_some() || self.shake_start.is_some() || self.spin.is_some()
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

        let mut resquare = false;
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
                if self.time - turn.start >= turn.duration + turn.hold {
                    resquare = turn.resquare;
                    self.turn = None;
                }
            }
        }

        if let Some(s) = &mut self.spin {
            // Follow the pointer closely but smoothly, so the gyro sees a
            // turn rather than jumps.
            s.angle += (s.target - s.angle) * (1.0 - (-25.0 * dt as f32).exp());
            self.pose.rotation = (Quat::from_axis_angle(s.axis, s.angle) * s.from).normalize();
        }

        if resquare {
            // Look at the face nearest the viewer, squared up.
            let front = self.viewer_right.cross(Vec3::Y);
            let toward = |f: Face| (self.pose.rotation * face_normal(f)).dot(front);
            let face = Face::ALL
                .into_iter()
                .max_by(|a, b| toward(*a).total_cmp(&toward(*b)))
                .unwrap_or(Face::PosZ);
            self.turn = Some(Turn {
                start: self.time,
                duration: 0.3,
                from: self.pose.rotation,
                to: self.squared_toward_viewer(face),
                hop: false,
                hold: 0.0,
                resquare: false,
            });
        }

        // A hand's tremor, while the die is held.
        if self.hand.tremor_dps > 0.0 && self.tumble.is_none() && self.shake_start.is_none() && !self.docked {
            let t = self.time as f32;
            let f = [9.0, 11.0, 7.0];
            let w: [f32; 3] = core::array::from_fn(|i| {
                self.hand.tremor_dps.to_radians()
                    * (std::f32::consts::TAU * f[i] * t + self.tremor_phase[i]).sin()
            });
            let v = Vec3::from_array(w) * dt as f32;
            self.pose.rotation = (Quat::from_scaled_axis(v) * self.pose.rotation).normalize();
        }

        if self.docked {
            // Settles into the Nest's pocket.
            self.pose.rotation = self.pose.rotation.slerp(self.dock_rot, ease_back);
            self.pose.position -= self.pose.position * ease_back;
            // The turn into place is what the gyro reads; a die set down
            // also gives the accelerometer a knock.
            if self.bump {
                self.bump = false;
                linear = Vec3::Y * 400.0;
            }
        } else if let Some(h) = self.hover {
            // Rising into the hand: the sensor reads the push.
            let gap = h - self.pose.position.y;
            self.pose.position.y += gap * ease_back;
            if gap > 0.05 {
                linear = Vec3::Y * 600.0;
            }
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
        let mut to = yaw * Quat::from_rotation_x(a) * Quat::from_rotation_y(b) * Quat::from_rotation_z(c);
        if let Some(up) = self.next_landing.take() {
            to = yaw * Quat::from_rotation_arc(face_normal(up), Vec3::Y);
        }
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
        let bias = Vec3::from_array(self.hand.gyro_bias_dps) * (std::f32::consts::PI / 180.0);
        let mdps = (omega_body + bias) * (180.0 / std::f32::consts::PI * 1000.0);
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
    fn a_spin_follows_the_pointer_and_settles_on_the_nearest_face() {
        let mut w = World::new();
        let before = w.pose().rotation;
        // Two and a bit faces to the left, in steps like pointer moves.
        for i in 1..=20 {
            assert!(w.spin(
                SpinAxis::Yaw,
                -2.3 * std::f32::consts::FRAC_PI_2 * i as f32 / 20.0,
                Vec3::X
            ));
            w.step(DT);
        }
        let samples = run(&mut w, 0.5);
        let mid = w.pose().rotation * before.inverse();
        assert!((mid.to_axis_angle().1 - 2.3 * std::f32::consts::FRAC_PI_2).abs() < 0.05);
        assert!(samples.iter().all(|s| s.gyro_mdps[1] <= 0), "one way only");
        w.end_spin();
        run(&mut w, 0.5);
        let (axis, angle) = (w.pose().rotation * before.inverse()).to_axis_angle();
        assert!(
            (angle - std::f32::consts::PI).abs() < 1e-3,
            "settles two faces on: {angle}"
        );
        assert!(axis.y.abs() > 0.99);
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
        assert!(w.on_charger());
    }

    #[test]
    fn docking_on_another_face_keeps_it_down_and_does_not_charge() {
        let mut w = World::new();
        w.place_face_up(Face::NegY); // the charging face up, so +Y is down
        run(&mut w, 1.0);
        w.set_docked(true);
        run(&mut w, 2.0);
        assert!(w.docked());
        assert!(!w.on_charger());
        assert!((w.pose().rotation * Vec3::Y).y < -0.99, "+Y stays down");
    }

    #[test]
    fn the_charging_face_charges_in_any_of_its_four_rotations() {
        for turns in 0..4 {
            let mut w = World::new();
            w.rotate(turns as f32 * std::f32::consts::FRAC_PI_2, 0.0);
            w.set_docked(true);
            run(&mut w, 2.0);
            assert!(w.on_charger(), "{turns} quarter turns");
        }
    }
}
