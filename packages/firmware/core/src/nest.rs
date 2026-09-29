//! The Nest: docking, charging and placement guidance (DOCK_BRIEF).
//!
//! Putting the die in the Nest should feel effortless. Wrong, the die says
//! how to fix it; right, it greets the Nest and becomes a bedside clock and
//! battery display.
//!
//! ```text
//!             lifted                                   lifted
//!   ┌──────────────────────────┐        ┌───────────────────────────────────┐
//!   ▼                          │        │                                   │
//! OFF_NEST ──(still ∧ near)──► SEATING (0.5 s) ──┬──► OK ──► DISPLAY
//!                                                ├──► WRONG_FACE
//!                                                └──► NO_POWER
//! ```
//!
//! [`Nest`] is pure: it is fed readings and the time, and says what changed
//! and what each face should show. The firmware does the hardware.

use libm::{cosf, floorf, sinf};
use smokebomb_hal::{ChargeState, Face, HapticEffect};

use crate::gfx::K;
use crate::orientation::{Quarter, BASES};

/// The die's charging face (the lid), in its own frame: −Y, which is the
/// bottom in the resting pose.
pub const CHARGE_FACE: Face = Face::NegY;

/// The field must exceed this on entry and fall below [`NEAR_EXIT_MG`] to
/// leave, so a reading at the edge doesn't flicker.
pub const NEAR_ENTER_MG: f32 = 5_000.0;
pub const NEAR_EXIT_MG: f32 = 3_000.0;
/// The field must point down within this angle (cosine of 35°): it rejects
/// phones, speakers and fridge magnets beside the die.
const NEAR_COS: f32 = 0.819_152;
pub const NEAR_DEBOUNCE_MS: u64 = 200;
/// Contacts and sensors settle before the die decides what it's seeing.
pub const SEATING_MS: u64 = 500;
/// Seated the right way up with no power for this long is NO_POWER.
pub const NO_POWER_WAIT_MS: u64 = 2_000;
/// The power chip's charger-input flag is debounced this long.
pub const VBUS_DEBOUNCE_MS: u64 = 150;
/// Motion this long counts as a pickup.
pub const LIFT_MS: u64 = 150;
pub const DOCK_ANIM_MS: u64 = 1_600;
/// Undock: the battery holds this long before the wake label.
pub const UNDOCK_HOLD_MS: u64 = 1_500;
/// A tap or pickup lights the screens this long at full brightness.
pub const AWAKE_MS: u64 = 10_000;
const DIM: f32 = 0.3;
const WRONG_DIM_MS: u64 = 60_000;
const WRONG_OFF_MS: u64 = 5 * 60_000;
const NO_POWER_DIM_MS: u64 = 30_000;
const NO_POWER_OFF_MS: u64 = 2 * 60_000;
const DISPLAY_DIM_MS: u64 = 2 * 60_000;
const SHIFT_EVERY_MS: u64 = 60_000;
/// When the fill reaches its level, the light tick plays.
const TICK_AT_MS: u64 = 1_100;
const FLASH_MS: u64 = 250;

/// The default night hours: local 23:00 to 07:00.
pub const NIGHT_START_H: u8 = 23;
pub const NIGHT_END_H: u8 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    OffNest,
    Seating { since: u64 },
    Ok { since: u64 },
    Wrong { since: u64 },
    NoPower { since: u64 },
    Display { since: u64 },
}

impl Phase {
    /// One of the docked states (the die's mode is Nest).
    pub fn docked(self) -> bool {
        !matches!(self, Phase::OffNest | Phase::Seating { .. })
    }
}

/// What a tick changed, for the firmware to act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub change: Change,
    pub haptic: Option<HapticEffect>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Change {
    #[default]
    None,
    /// A docked state began (from seating).
    Docked,
    /// The die was lifted from a docked state. `animate` is whether the
    /// undock animation plays (it does from OK and DISPLAY).
    Undocked { animate: bool },
}

/// One tick's inputs.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub now: u64,
    /// Motion below threshold for 300 ms.
    pub still: bool,
    /// The face up, from gravity.
    pub up_face: Face,
    /// The power chip's charger-input flag, not yet debounced.
    pub vbus: bool,
    pub charge: ChargeState,
}

/// Magnetometer sampling: only once the die comes to rest, every 100 ms for
/// the first second, every 500 ms until 3 s, then every 5 s. Off in motion.
#[derive(Clone, Copy, Debug, Default)]
struct Sampler {
    still_since: Option<u64>,
    last: Option<u64>,
}

impl Sampler {
    fn due(&mut self, now: u64, still: bool, asleep: bool) -> bool {
        if !still || asleep {
            *self = Self::default();
            return false;
        }
        let since = *self.still_since.get_or_insert(now);
        let every = match now - since {
            0..=999 => 100,
            1_000..=2_999 => 500,
            _ => 5_000,
        };
        if self.last.is_none_or(|l| now - l >= every) {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

/// `nestNear`: strong enough (with hysteresis), pointing down, and steady
/// for 200 ms.
#[derive(Clone, Copy, Debug, Default)]
struct Near {
    strong: bool,
    since: Option<u64>,
    near: bool,
}

impl Near {
    fn feed(&mut self, now: u64, b_mg: [i32; 3], up: [f32; 3]) {
        let b_mg = b_mg.map(|v| v as f32);
        let mag = libm::sqrtf(dot(b_mg, b_mg));
        let limit = if self.strong { NEAR_EXIT_MG } else { NEAR_ENTER_MG };
        self.strong = mag > limit;
        // The magnet is below: the field points along gravity, away from up.
        let un = libm::sqrtf(dot(up, up));
        let down = self.strong && un > 1.0 && -dot(b_mg, up) / (mag * un) > NEAR_COS;
        if down {
            let since = *self.since.get_or_insert(now);
            self.near = now - since >= NEAR_DEBOUNCE_MS;
        } else {
            self.since = None;
            self.near = false;
        }
    }
}

/// A flag that follows its input only once the input has held still.
#[derive(Clone, Copy, Debug, Default)]
struct Debounced {
    value: bool,
    pending_since: Option<u64>,
}

impl Debounced {
    fn update(&mut self, raw: bool, now: u64) -> bool {
        if raw == self.value {
            self.pending_since = None;
        } else if now - *self.pending_since.get_or_insert(now) >= VBUS_DEBOUNCE_MS {
            self.value = raw;
            self.pending_since = None;
        }
        self.value
    }
}

#[derive(Clone, Copy, Debug)]
struct Undock {
    start: u64,
    top: Face,
    down: Face,
}

pub struct Nest {
    phase: Phase,
    sampler: Sampler,
    near: Near,
    vbus: Debounced,
    moving_since: Option<u64>,
    /// The last tap, pickup or arrival: dimming counts from here.
    wake_at: u64,
    /// The face pointing down while docked.
    down: Face,
    tick_played: bool,
    prev_charge: ChargeState,
    full_flash: Option<u64>,
    undock: Option<Undock>,
    /// The seconds counter last seen, and when it changed: makes the
    /// second hand smooth between whole seconds.
    sec: Option<(u32, u64)>,
}

impl Default for Nest {
    fn default() -> Self {
        Self::new()
    }
}

impl Nest {
    pub const fn new() -> Self {
        Self {
            phase: Phase::OffNest,
            sampler: Sampler {
                still_since: None,
                last: None,
            },
            near: Near {
                strong: false,
                since: None,
                near: false,
            },
            vbus: Debounced {
                value: false,
                pending_since: None,
            },
            moving_since: None,
            wake_at: 0,
            down: CHARGE_FACE,
            tick_played: false,
            prev_charge: ChargeState::Idle,
            full_flash: None,
            undock: None,
            sec: None,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn docked(&self) -> bool {
        self.phase.docked()
    }

    /// The face pointing down while docked.
    pub fn down(&self) -> Face {
        self.down
    }

    /// Debounced `nestNear`.
    pub fn near(&self) -> bool {
        self.near.near
    }

    /// Whether the firmware should read the magnetometer now. Docked, the
    /// answer is settled until the die is lifted.
    pub fn wants_reading(&mut self, now: u64, still: bool, asleep: bool) -> bool {
        let due = self.sampler.due(now, still, asleep);
        due && !self.phase.docked()
    }

    /// A magnetometer reading, hard iron already subtracted (milligauss),
    /// with the gravity-up vector (any length).
    pub fn reading(&mut self, now: u64, b_mg: [i32; 3], up: [f32; 3]) {
        self.near.feed(now, b_mg, up);
    }

    /// A tap or touch: the screens light for [`AWAKE_MS`].
    pub fn touch(&mut self, now: u64) {
        self.wake_at = now;
    }

    /// The die was picked up while docked or waiting to dock.
    fn lift_reset(&mut self) {
        self.near = Near::default();
        self.sampler = Sampler::default();
    }

    pub fn update(&mut self, i: Inputs) -> Step {
        let now = i.now;
        let vbus = self.vbus.update(i.vbus, now);
        if i.still {
            self.moving_since = None;
        } else {
            self.moving_since.get_or_insert(now);
        }
        let lifted = self.moving_since.is_some_and(|t| now - t >= LIFT_MS);
        let mut step = Step::default();

        if self.undock.is_some_and(|u| now - u.start >= UNDOCK_HOLD_MS) {
            self.undock = None;
        }

        match self.phase {
            Phase::OffNest => {
                if i.still && self.near.near {
                    self.phase = Phase::Seating { since: now };
                }
            }
            Phase::Seating { since } => {
                if !i.still {
                    self.phase = Phase::OffNest;
                    self.lift_reset();
                } else if now - since >= SEATING_MS {
                    let down = i.up_face.opposite();
                    self.down = down;
                    if down != CHARGE_FACE {
                        self.arrive(Phase::Wrong { since: now }, &mut step, HapticEffect::DoubleTap);
                    } else if vbus {
                        self.arrive(Phase::Ok { since: now }, &mut step, HapticEffect::SeatThunk);
                    } else if now - since >= SEATING_MS + NO_POWER_WAIT_MS {
                        self.arrive(Phase::NoPower { since: now }, &mut step, HapticEffect::SoftBuzz);
                    }
                }
            }
            Phase::Ok { since } => {
                if lifted {
                    self.leave(now, true, &mut step);
                } else if !vbus {
                    self.phase = Phase::NoPower { since: now };
                    step.haptic = Some(HapticEffect::SoftBuzz);
                    self.wake_at = now;
                } else {
                    if !self.tick_played && now - since >= TICK_AT_MS {
                        self.tick_played = true;
                        step.haptic = Some(HapticEffect::DockTick);
                    }
                    if now - since >= DOCK_ANIM_MS {
                        self.phase = Phase::Display { since: now };
                    }
                }
            }
            Phase::Display { .. } => {
                if lifted {
                    self.leave(now, true, &mut step);
                } else if !vbus {
                    self.phase = Phase::NoPower { since: now };
                    step.haptic = Some(HapticEffect::SoftBuzz);
                    self.wake_at = now;
                } else if self.prev_charge != ChargeState::Full && i.charge == ChargeState::Full {
                    self.full_flash = Some(now);
                }
            }
            Phase::Wrong { .. } => {
                if lifted {
                    self.leave(now, false, &mut step);
                }
            }
            Phase::NoPower { .. } => {
                if lifted {
                    self.leave(now, false, &mut step);
                } else if vbus {
                    self.phase = Phase::Ok { since: now };
                    self.tick_played = false;
                    step.haptic = Some(HapticEffect::SeatThunk);
                    self.wake_at = now;
                }
            }
        }
        self.prev_charge = i.charge;
        step
    }

    fn arrive(&mut self, phase: Phase, step: &mut Step, haptic: HapticEffect) {
        self.phase = phase;
        self.tick_played = false;
        self.full_flash = None;
        if let Phase::Ok { since } | Phase::Wrong { since } | Phase::NoPower { since } = phase {
            self.wake_at = since;
        }
        step.change = Change::Docked;
        step.haptic = Some(haptic);
    }

    fn leave(&mut self, now: u64, animate: bool, step: &mut Step) {
        let down = self.down;
        self.phase = Phase::OffNest;
        self.lift_reset();
        self.full_flash = None;
        if animate {
            self.undock = Some(Undock {
                start: now,
                top: down.opposite(),
                down,
            });
            step.haptic = Some(HapticEffect::Ready);
        }
        step.change = Change::Undocked { animate };
    }

    /// Left the docked states some other way (a menu opened while docked
    /// keeps them; this is for the firmware overriding).
    pub fn undocking(&self) -> bool {
        self.undock.is_some()
    }

    /// The time of day with a smooth second hand: the RTC counts whole
    /// seconds, so the fraction runs from when the count last changed.
    pub fn clock_time(&mut self, now: u64, local_seconds: u32) -> f32 {
        let frac = match &mut self.sec {
            Some((s, at)) if *s == local_seconds => (now.saturating_sub(*at) as f32 / 1000.0).min(0.999),
            slot => {
                let known = slot.is_some();
                *slot = Some((local_seconds, now));
                if known {
                    0.0
                } else {
                    0.5
                }
            }
        };
        local_seconds as f32 + frac
    }

    /// How bright the screens are (0 = off), by state and time.
    fn brightness(&self, now: u64, secs: f32, night: (u8, u8)) -> f32 {
        let idle = now.saturating_sub(self.wake_at);
        match self.phase {
            Phase::Wrong { .. } => match idle {
                0..WRONG_DIM_MS => 1.0,
                WRONG_DIM_MS..WRONG_OFF_MS => DIM,
                _ => 0.0,
            },
            Phase::NoPower { .. } => match idle {
                0..NO_POWER_DIM_MS => 1.0,
                NO_POWER_DIM_MS..NO_POWER_OFF_MS => DIM,
                _ => 0.0,
            },
            Phase::Display { .. } => {
                if idle < AWAKE_MS {
                    1.0
                } else if is_night(secs, night) {
                    0.0
                } else if idle >= DISPLAY_DIM_MS {
                    DIM
                } else {
                    1.0
                }
            }
            _ => 1.0,
        }
    }

    /// What `face` shows in a docked state (never blank because of a
    /// missing state: callers only ask while [`Nest::docked`]).
    pub fn face(&self, now: u64, face: Face, env: &Env) -> NestFace {
        let level = env.battery.min(100) as f32 / 100.0;
        let (charge_face_down, top) = (self.down == CHARGE_FACE, self.down.opposite());
        let dim = self.brightness(now, env.secs, env.night);
        let blank = NestFace::new(Screen::Blank);
        let fault = env.charge == ChargeState::Fault;
        let label = if env.charge == ChargeState::Full {
            Label::Full
        } else {
            Label::Charging
        };
        let quarter = env.quarters[face.index()];
        let mut out = match self.phase {
            Phase::Ok { since } => {
                let t = now.saturating_sub(since) as f32 / 1000.0;
                if fault {
                    fault_face(face, top, env.secs)
                } else if face == top {
                    NestFace::new(Screen::Charge(ok_charge(t, level, label, env)))
                } else if face == self.down {
                    blank
                } else {
                    NestFace::new(Screen::Clock(ok_clock(t, env)))
                }
            }
            Phase::Display { since } => if fault {
                fault_face(face, top, env.secs)
            } else if face == top {
                let flash = self
                    .full_flash
                    .map_or(0.0, |t| 1.0 - now.saturating_sub(t) as f32 / FLASH_MS as f32)
                    .max(0.0);
                NestFace::new(Screen::Charge(ChargeView {
                    fill: level,
                    pct: level * 100.0,
                    label,
                    alpha: 1.0,
                    text: 1.0,
                    flash,
                    wave: now as f32 / 1000.0,
                }))
            } else if face == self.down {
                blank
            } else {
                NestFace::new(Screen::Clock(ClockView::steady(env.secs)))
            }
            .shifted(shift_at(now.saturating_sub(since))),
            Phase::NoPower { .. } => {
                if face == top {
                    NestFace::new(Screen::NoPower)
                } else {
                    blank
                }
            }
            Phase::Wrong { .. } if charge_face_down => blank,
            Phase::Wrong { .. } => self.wrong(now, face, top, quarter, env),
            _ => blank,
        };
        out.dim *= dim;
        out
    }

    fn wrong(&self, now: u64, face: Face, top: Face, quarter: Quarter, env: &Env) -> NestFace {
        let t = now as f32 / 1000.0;
        let c = CHARGE_FACE;
        let toward = |from: Face| toward(from, c, env.quarters[from.index()]);
        let _ = quarter;
        if face == self.down {
            return NestFace::new(Screen::Blank);
        }
        if c == top {
            // The charging face is on top: flip the die over.
            return if face == top {
                NestFace::new(Screen::Flip { t })
            } else {
                NestFace::new(Screen::SideDown { pulse: pulse(t) })
            };
        }
        // The charging face is on a side: tip toward it.
        if face == top {
            NestFace::new(Screen::TipArrow {
                dir: toward(top),
                nudge: nudge(t),
            })
        } else if face == c {
            NestFace::new(Screen::FaceDown)
        } else if face == c.opposite() {
            NestFace::new(Screen::Blank)
        } else {
            NestFace::new(Screen::Toward { dir: toward(face) })
        }
    }

    /// What `face` shows while the die has just been lifted from the Nest
    /// (the undock animation and the battery hold), if anything.
    pub fn undock_face(&self, now: u64, face: Face, env: &Env) -> Option<NestFace> {
        let u = self.undock?;
        let t = now.saturating_sub(u.start) as f32 / 1000.0;
        if face == u.down {
            return Some(NestFace::new(Screen::Blank));
        }
        let level = env.battery.min(100) as f32 / 100.0;
        Some(if face == u.top {
            let fade = 1.0 - ((t - 1.2) / 0.3).clamp(0.0, 1.0);
            NestFace::new(Screen::Charge(ChargeView {
                fill: level,
                pct: level * 100.0,
                label: Label::Battery,
                alpha: fade,
                text: 1.0,
                flash: 0.0,
                wave: t,
            }))
        } else {
            let spin = ease((t / 0.5).min(1.0));
            let dissolve = ((t - 0.4) / 0.4).clamp(0.0, 1.0);
            if dissolve >= 1.0 {
                NestFace::new(Screen::Blank)
            } else {
                NestFace::new(Screen::Clock(ClockView {
                    secs: env.secs,
                    alpha: 1.0 - dissolve,
                    ticks: 1.0,
                    hands: 1.0,
                    spin,
                    smoke: 0.0,
                    puff: (t > 0.4).then_some(dissolve),
                }))
            }
        })
    }
}

/// What the screens need from the rest of the die.
pub struct Env<'a> {
    pub battery: u8,
    pub charge: ChargeState,
    /// Local time of day, with the second hand's fraction.
    pub secs: f32,
    /// Night hours (start, end), local hours.
    pub night: (u8, u8),
    pub reduced_motion: bool,
    /// Each face's text orientation.
    pub quarters: &'a [Quarter; 6],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NestFace {
    pub screen: Screen,
    /// Brightness, 0 = off, 0.3 = dimmed.
    pub dim: f32,
    /// Burn-in shift, canvas units.
    pub shift: (f32, f32),
}

impl NestFace {
    fn new(screen: Screen) -> Self {
        Self {
            screen,
            dim: 1.0,
            shift: (0.0, 0.0),
        }
    }

    fn shifted(mut self, shift: (f32, f32)) -> Self {
        self.shift = shift;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Label {
    Charging,
    Full,
    Battery,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChargeView {
    /// Fill level, 0–1.
    pub fill: f32,
    /// The percentage shown (it counts up while docking).
    pub pct: f32,
    pub label: Label,
    /// The whole screen's opacity.
    pub alpha: f32,
    /// The text's opacity.
    pub text: f32,
    /// The full flash, 1 → 0.
    pub flash: f32,
    /// Seconds, for the fill's wave.
    pub wave: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockView {
    /// Local time of day in seconds.
    pub secs: f32,
    pub alpha: f32,
    /// Tick opacity, 0–1.
    pub ticks: f32,
    /// How far the hands have swept from 12:00 to the time, 0–1.
    pub hands: f32,
    /// Extra forward turn on undock, 0–1 of a full turn.
    pub spin: f32,
    /// The sinking wisp, 0–1 (docking).
    pub smoke: f32,
    /// The dissolving puff, 0–1 (undocking).
    pub puff: Option<f32>,
}

impl ClockView {
    pub fn steady(secs: f32) -> Self {
        Self {
            secs,
            alpha: 1.0,
            ticks: 1.0,
            hands: 1.0,
            spin: 0.0,
            smoke: 0.0,
            puff: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Screen {
    Blank,
    Charge(ChargeView),
    Clock(ClockView),
    /// Charging face on top: the flip-over icon and "Flip me over".
    Flip {
        t: f32,
    },
    /// A side face when the charging face is on top: chevron up and
    /// "This side down". `pulse` is the chevron's opacity.
    SideDown {
        pulse: f32,
    },
    /// Top face, charging face on a side: an arrow at that edge. `dir` is
    /// in content coordinates; `nudge` is 0–1 toward the edge.
    TipArrow {
        dir: (f32, f32),
        nudge: f32,
    },
    /// The charging face itself, facing sideways: ↓ and "This side down".
    FaceDown,
    /// A face beside the charging face: a chevron pointing at it.
    Toward {
        dir: (f32, f32),
    },
    NoPower,
    Fault,
}

fn fault_face(face: Face, top: Face, secs: f32) -> NestFace {
    if face == top {
        NestFace::new(Screen::Fault)
    } else if face == top.opposite() {
        NestFace::new(Screen::Blank)
    } else {
        let mut f = NestFace::new(Screen::Clock(ClockView::steady(secs)));
        f.dim = DIM;
        f
    }
}

fn ok_charge(t: f32, level: f32, label: Label, env: &Env) -> ChargeView {
    if env.reduced_motion {
        let a = (t / 0.3).min(1.0);
        return ChargeView {
            fill: level,
            pct: level * 100.0,
            label,
            alpha: a,
            text: 1.0,
            flash: 0.0,
            wave: t,
        };
    }
    ChargeView {
        fill: level * ease((t - 0.3) / 0.8),
        pct: level * 100.0 * ease((t - 0.7) / 0.5),
        label,
        alpha: 1.0,
        text: ((t - 0.7) / 0.2).clamp(0.0, 1.0),
        flash: 0.0,
        wave: t,
    }
}

fn ok_clock(t: f32, env: &Env) -> ClockView {
    if env.reduced_motion {
        return ClockView {
            alpha: (t / 0.3).min(1.0),
            ..ClockView::steady(env.secs)
        };
    }
    ClockView {
        secs: env.secs,
        alpha: 1.0,
        ticks: ((t - 0.3) / 0.3).clamp(0.0, 1.0),
        hands: ease((t - 0.5) / 0.6),
        spin: 0.0,
        smoke: if t < 0.5 { t / 0.5 } else { 0.0 },
        puff: None,
    }
}

/// Ease-out cubic, clamped.
fn ease(x: f32) -> f32 {
    let u = 1.0 - x.clamp(0.0, 1.0);
    1.0 - u * u * u
}

/// The chevron's opacity: 40% → 100% on a 1 s period.
fn pulse(t: f32) -> f32 {
    0.4 + 0.6 * (0.5 + 0.5 * sinf(core::f32::consts::TAU * t))
}

/// The arrow's nudge toward the edge, 0–1, on a 0.8 s ease-in-out loop.
fn nudge(t: f32) -> f32 {
    let p = t / 0.8;
    let u = p - floorf(p);
    0.5 - 0.5 * cosf(core::f32::consts::TAU * u)
}

/// One pixel up, right, down, left, a step a minute.
fn shift_at(since_ms: u64) -> (f32, f32) {
    let px = 1.0 / K;
    match (since_ms / SHIFT_EVERY_MS) % 4 {
        0 => (0.0, -px),
        1 => (px, 0.0),
        2 => (0.0, px),
        _ => (-px, 0.0),
    }
}

/// Whether `secs` (local time of day) falls in the night hours.
pub fn is_night(secs: f32, (start, end): (u8, u8)) -> bool {
    let (s, e) = (start as f32 * 3600.0, end as f32 * 3600.0);
    if s == e {
        false
    } else if s < e {
        secs >= s && secs < e
    } else {
        secs >= s || secs < e
    }
}

/// The direction from `from` toward the adjacent face `to`, in `from`'s
/// content coordinates (x right, y down) for a face drawn with `quarter`.
pub fn toward(from: Face, to: Face, quarter: Quarter) -> (f32, f32) {
    let b = &BASES[from.index()];
    let n = BASES[to.index()].n;
    quarter.unmap_dir(dot(n, b.x), -dot(n, b.y))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still(now: u64, up: Face, vbus: bool) -> Inputs {
        Inputs {
            now,
            still: true,
            up_face: up,
            vbus,
            charge: ChargeState::Charging,
        }
    }

    /// Drive a nest that already feels the magnet, from `t0`, until it has
    /// decided; returns the time.
    fn seat(n: &mut Nest, up: Face, vbus: bool, until: u64) -> u64 {
        n.reading(0, [0, -12_000, 0], [0.0, 1000.0, 0.0]);
        n.reading(250, [0, -12_000, 0], [0.0, 1000.0, 0.0]);
        let mut t = 300;
        while t < until {
            n.update(still(t, up, vbus));
            t += 16;
        }
        t
    }

    #[test]
    fn near_needs_strength_direction_and_200_ms() {
        let up = [0.0, 1000.0, 0.0];
        let mut n = Near::default();
        n.feed(0, [0, -12_000, 0], up);
        assert!(!n.near, "not yet steady");
        n.feed(200, [0, -12_000, 0], up);
        assert!(n.near);
        // Hysteresis: 4 G still counts once in, 2.9 G does not.
        n.feed(300, [0, -4_000, 0], up);
        assert!(n.near);
        n.feed(400, [0, -2_900, 0], up);
        assert!(!n.near);
        // 4 G is not enough to get in.
        let mut n = Near::default();
        n.feed(0, [0, -4_000, 0], up);
        n.feed(300, [0, -4_000, 0], up);
        assert!(!n.near);
    }

    #[test]
    fn a_stray_magnet_beside_the_die_is_rejected() {
        let up = [0.0, 1000.0, 0.0];
        let mut n = Near::default();
        for t in (0..1000).step_by(100) {
            n.feed(t, [15_000, 0, 0], up); // sideways
            n.feed(t, [0, 15_000, 0], up); // above: magnet overhead
        }
        assert!(!n.near);
        // Tilted 30° off straight down is fine, 40° is not.
        let mut n = Near::default();
        let b = |deg: f32| {
            let a = deg.to_radians();
            [(12_000.0 * sinf(a)) as i32, (-12_000.0 * cosf(a)) as i32, 0]
        };
        n.feed(0, b(30.0).map(|v| v), up);
        n.feed(250, b(30.0).map(|v| v), up);
        assert!(n.near);
        let mut n = Near::default();
        n.feed(0, b(40.0).map(|v| v), up);
        n.feed(250, b(40.0).map(|v| v), up);
        assert!(!n.near);
    }

    #[test]
    fn the_sampler_slows_down_and_stops_in_motion() {
        let mut s = Sampler::default();
        let count =
            |s: &mut Sampler, from: u64, to: u64| (from..to).filter(|&t| s.due(t, true, false)).count();
        assert_eq!(count(&mut s, 0, 1_000), 10);
        assert_eq!(count(&mut s, 1_000, 3_000), 4);
        assert_eq!(count(&mut s, 3_000, 13_000), 2);
        assert!(!s.due(13_001, false, false));
        assert!(s.due(13_002, true, false), "restarts at once when still again");
        assert!(!s.due(13_003, true, true), "off while asleep");
    }

    #[test]
    fn right_face_and_power_docks_after_seating() {
        let mut n = Nest::new();
        n.reading(0, [0, -12_000, 0], [0.0, 1000.0, 0.0]);
        n.reading(250, [0, -12_000, 0], [0.0, 1000.0, 0.0]);
        let mut changes = std::vec::Vec::new();
        let mut haptics = std::vec::Vec::new();
        for t in (300..3_000).step_by(16) {
            let s = n.update(still(t, Face::PosY, true));
            if s.change != Change::None {
                changes.push((t, s.change));
            }
            if let Some(h) = s.haptic {
                haptics.push(h);
            }
        }
        assert_eq!(changes.len(), 1);
        let (t, c) = changes[0];
        assert_eq!(c, Change::Docked);
        assert!((800..830).contains(&t), "0.5 s after it went still: {t}");
        assert_eq!(haptics, [HapticEffect::SeatThunk, HapticEffect::DockTick]);
        assert!(matches!(n.phase(), Phase::Display { .. }), "after 1.6 s");
    }

    #[test]
    fn wrong_face_guides_and_a_lift_clears_it() {
        let mut n = Nest::new();
        let t = seat(&mut n, Face::NegX, false, 900);
        assert!(matches!(n.phase(), Phase::Wrong { .. }));
        assert_eq!(n.down(), Face::PosX);
        let mut moving = still(t, Face::NegX, false);
        moving.still = false;
        let mut last = Step::default();
        for k in 0..20 {
            moving.now = t + k * 16;
            let s = n.update(moving);
            if s.change != Change::None {
                last = s;
            }
        }
        assert_eq!(last.change, Change::Undocked { animate: false });
        assert_eq!(last.haptic, None);
        assert_eq!(n.phase(), Phase::OffNest);
        assert!(!n.near());
    }

    #[test]
    fn no_power_waits_two_seconds_and_power_arriving_late_docks() {
        let mut n = Nest::new();
        seat(&mut n, Face::PosY, false, 2_400);
        assert!(!n.docked(), "still deciding");
        seat_more(&mut n, 2_400, 3_000, false);
        assert!(matches!(n.phase(), Phase::NoPower { .. }));
        seat_more(&mut n, 3_000, 3_400, true);
        assert!(matches!(n.phase(), Phase::Ok { .. }));
    }

    fn seat_more(n: &mut Nest, from: u64, to: u64, vbus: bool) {
        for t in (from..to).step_by(16) {
            n.update(still(t, Face::PosY, vbus));
        }
    }

    #[test]
    fn a_pulled_cable_while_displaying_goes_to_no_power() {
        let mut n = Nest::new();
        seat(&mut n, Face::PosY, true, 2_600);
        assert!(matches!(n.phase(), Phase::Display { .. }));
        seat_more(&mut n, 2_600, 3_000, false);
        assert!(matches!(n.phase(), Phase::NoPower { .. }));
    }

    #[test]
    fn night_hours_wrap_midnight() {
        let h = |x: f32| x * 3600.0;
        assert!(is_night(h(23.5), (23, 7)));
        assert!(is_night(h(3.0), (23, 7)));
        assert!(!is_night(h(7.0), (23, 7)));
        assert!(!is_night(h(12.0), (23, 7)));
        assert!(is_night(h(1.0), (0, 6)));
    }

    #[test]
    fn arrows_point_at_the_right_edge() {
        // +Z drawn upright: +Y is up (canvas −y), +X is right.
        assert_eq!(toward(Face::PosZ, Face::PosY, Quarter::R0), (0.0, -1.0));
        assert_eq!(toward(Face::PosZ, Face::PosX, Quarter::R0), (1.0, 0.0));
        // Drawn a quarter turn clockwise, the arrow's content direction
        // turns back so it still points the same physical way.
        let (x, y) = toward(Face::PosZ, Face::PosY, Quarter::R90);
        assert_eq!(Quarter::R90.unmap_dir(0.0, -1.0), (x, y));
        // Round trip through the panel mapping.
        for q in [Quarter::R0, Quarter::R90, Quarter::R180, Quarter::R270] {
            let (cx, cy) = q.unmap_dir(0.0, -1.0);
            let (x, y) = ((47.5 + cx * 10.0) as usize, (47.5 + cy * 10.0) as usize);
            let (px, py) = q.map(x, y);
            // Pixel indices, not centres: allow the half-pixel.
            assert!(
                (47..=48).contains(&px) && (37..=38).contains(&py),
                "{q:?}: {px},{py}"
            );
        }
    }

    #[test]
    fn dimming_follows_the_brief() {
        let mut n = Nest::new();
        seat(&mut n, Face::NegX, false, 900);
        let env = Env {
            battery: 50,
            charge: ChargeState::Idle,
            secs: 12.0 * 3600.0,
            night: (23, 7),
            reduced_motion: false,
            quarters: &[Quarter::R0; 6],
        };
        let b = |t| n.face(t, Face::PosY, &env).dim;
        assert_eq!(b(1_000 + 30_000), 1.0);
        assert_eq!(b(1_000 + 61_000), DIM);
        assert_eq!(b(1_000 + 5 * 60_000 + 2_000), 0.0);
    }
}
