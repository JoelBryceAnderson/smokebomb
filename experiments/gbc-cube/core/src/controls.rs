//! Phase 5: the cube as a joypad.
//!
//! | Input | Button |
//! |---|---|
//! | Tilt (past a deadzone; hold the tilt to keep walking) | D-pad, toward the edge that dips |
//! | Tap the up face | A |
//! | Tap a side face | B |
//! | Long-press the up face | Start |
//! | Shake | Select |
//!
//! Tilt directions are map directions: dip the cube's north edge (the edge
//! the map's north runs off) and the player walks north, whichever face is
//! up. Every threshold is in [`ControlConfig`].

use smokebomb_hal::Face;

use crate::buttons::Buttons;
use crate::geom::{Compass, Heading};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControlConfig {
    /// Tilt below this (degrees off level) does nothing.
    pub deadzone_deg: f32,
    /// Tilt past this is the cube being rolled onto another face, not
    /// walking: the D-pad lets go.
    pub walk_max_deg: f32,
    /// A tilt has to be held this long before it walks, so the moment a
    /// roll spends passing through the walking range doesn't take a step.
    pub walk_hold_ms: u32,
    /// To switch from walking one way to the other axis, the other axis's
    /// tilt has to be this many times bigger (no flicker on diagonals).
    pub axis_hysteresis: f32,
    /// A touch shorter than this is a tap.
    pub tap_max_ms: u32,
    /// Holding the up face this long presses Start (and the tap is dropped).
    pub long_press_ms: u32,
    /// How long a tap holds its button down. The game polls the joypad once
    /// a frame, so a few frames.
    pub pulse_ms: u32,
    /// Acceleration magnitude above this is a shake jolt (milli-g).
    pub shake_mg: u32,
    /// This many jolts...
    pub shake_jolts: u8,
    /// ...within this long is a shake.
    pub shake_window_ms: u32,
    /// No walking for this long after the up face changes, so the tilt that
    /// rolled the cube doesn't carry on as a step.
    pub roll_lockout_ms: u32,
}

impl Default for ControlConfig {
    fn default() -> Self {
        ControlConfig {
            deadzone_deg: 12.0,
            walk_max_deg: 40.0,
            walk_hold_ms: 120,
            axis_hysteresis: 1.3,
            tap_max_ms: 300,
            long_press_ms: 650,
            pulse_ms: 100,
            shake_mg: 1900,
            shake_jolts: 3,
            shake_window_ms: 800,
            roll_lockout_ms: 450,
        }
    }
}

/// One sample of the sensors.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sense {
    pub now_ms: u32,
    /// The sky in die axes (filtered gravity; any length).
    pub up: [f32; 3],
    /// Raw accelerometer, milli-g.
    pub accel_mg: [i16; 3],
    /// Bit `n` set while face `n` is touched.
    pub touch: u8,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    walking: Option<Compass>,
    /// Where the tilt points and since when, before it counts as walking.
    leaning: Option<(Compass, u32)>,
    touch_start: [Option<u32>; 6],
    long_fired: bool,
    pulses: [(Buttons, u32); 4],
    jolt_times: [u32; 8],
    jolt_count: usize,
    above: bool,
    shaking_until: u32,
    select_lock_until: u32,
    locked_until: u32,
}

impl Controls {
    pub fn new() -> Self {
        Self::default()
    }

    /// The up face just changed: stop walking for a moment.
    pub fn rolled(&mut self, now_ms: u32, cfg: &ControlConfig) {
        self.walking = None;
        self.leaning = None;
        self.locked_until = now_ms + cfg.roll_lockout_ms;
    }

    /// Which way the tilt says to walk, if any.
    pub fn walking(&self) -> Option<Compass> {
        self.walking
    }

    pub fn update(&mut self, s: &Sense, heading: &Heading, cfg: &ControlConfig) -> Buttons {
        let mut out = Buttons::NONE;
        self.shake(s, cfg);
        self.tilt(s, heading, cfg);
        if let Some(c) = self.walking {
            out |= match c {
                Compass::North => Buttons::UP,
                Compass::South => Buttons::DOWN,
                Compass::East => Buttons::RIGHT,
                Compass::West => Buttons::LEFT,
            };
        }
        self.touches(s, heading, cfg);
        for (b, until) in &mut self.pulses {
            if !b.is_empty() && s.now_ms < *until {
                out |= *b;
            } else {
                *b = Buttons::NONE;
            }
        }
        out
    }

    fn pulse(&mut self, b: Buttons, now: u32, cfg: &ControlConfig) {
        let i = self
            .pulses
            .iter()
            .position(|(p, until)| p.is_empty() || now >= *until)
            .unwrap_or(0);
        self.pulses[i] = (b, now + cfg.pulse_ms);
    }

    fn tilt(&mut self, s: &Sense, h: &Heading, cfg: &ControlConfig) {
        let len = libm::sqrtf(s.up.iter().map(|c| c * c).sum());
        if len < 1e-3 || s.now_ms < self.locked_until || s.now_ms < self.shaking_until {
            self.walking = None;
            return;
        }
        let along = |a: [i8; 3]| (0..3).map(|k| s.up[k] * a[k] as f32).sum::<f32>() / len;
        // The sky leans toward the raised edge, so the dipped edge is the
        // other way.
        let east = -along(h.east());
        let north = -along(h.north);
        let tilt = libm::sqrtf(east * east + north * north);
        let walk = libm::sinf(cfg.deadzone_deg.to_radians())..=libm::sinf(cfg.walk_max_deg.to_radians());
        if !walk.contains(&tilt) {
            self.walking = None;
            self.leaning = None;
            return;
        }
        let (ae, an) = (libm::fabsf(east), libm::fabsf(north));
        let ns = if north > 0.0 {
            Compass::North
        } else {
            Compass::South
        };
        let ew = if east > 0.0 { Compass::East } else { Compass::West };
        let dir = match self.walking.or(self.leaning.map(|l| l.0)) {
            Some(Compass::North | Compass::South) if an * cfg.axis_hysteresis >= ae => ns,
            Some(Compass::East | Compass::West) if ae * cfg.axis_hysteresis >= an => ew,
            _ if an >= ae => ns,
            _ => ew,
        };
        if self.walking.is_some() {
            self.walking = Some(dir);
            return;
        }
        match self.leaning {
            Some((d, since)) if d == dir => {
                if s.now_ms.wrapping_sub(since) >= cfg.walk_hold_ms {
                    self.walking = Some(dir);
                    self.leaning = None;
                }
            }
            _ => self.leaning = Some((dir, s.now_ms)),
        }
    }

    fn touches(&mut self, s: &Sense, h: &Heading, cfg: &ControlConfig) {
        let up = h.up_face().index();
        let down = h.down_face().index();
        for f in Face::ALL {
            let i = f.index();
            if i == down {
                self.touch_start[i] = None; // resting on the table
                continue;
            }
            let held = s.touch & (1 << i) != 0;
            match (self.touch_start[i], held) {
                (None, true) => {
                    self.touch_start[i] = Some(s.now_ms);
                    if i == up {
                        self.long_fired = false;
                    }
                }
                (Some(t0), true) => {
                    if i == up && !self.long_fired && s.now_ms - t0 >= cfg.long_press_ms {
                        self.long_fired = true;
                        self.pulse(Buttons::START, s.now_ms, cfg);
                    }
                }
                (Some(t0), false) => {
                    self.touch_start[i] = None;
                    let tap = s.now_ms - t0 <= cfg.tap_max_ms;
                    if i == up {
                        if tap && !self.long_fired {
                            self.pulse(Buttons::A, s.now_ms, cfg);
                        }
                    } else if tap {
                        self.pulse(Buttons::B, s.now_ms, cfg);
                    }
                }
                (None, false) => {}
            }
        }
    }

    fn shake(&mut self, s: &Sense, cfg: &ControlConfig) {
        let a = s.accel_mg.map(|c| c as i64);
        let mag2 = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]) as u64;
        let over = mag2 > (cfg.shake_mg as u64).pow(2);
        if over && !self.above {
            // A new jolt.
            let n = self.jolt_times.len();
            self.jolt_times[self.jolt_count % n] = s.now_ms;
            self.jolt_count += 1;
            let need = cfg.shake_jolts.max(1) as usize;
            if self.jolt_count >= need {
                let first = self.jolt_times[(self.jolt_count - need) % n];
                if s.now_ms - first <= cfg.shake_window_ms && s.now_ms >= self.select_lock_until {
                    self.pulse(Buttons::SELECT, s.now_ms, cfg);
                    self.jolt_count = 0;
                    // One Select per shake; let the cube settle before tilt
                    // means walking again.
                    self.select_lock_until = s.now_ms + 1000;
                    self.shaking_until = s.now_ms + 1000;
                }
            }
        }
        if over {
            self.shaking_until = self.shaking_until.max(s.now_ms + 250);
        }
        self.above = over;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Heading;

    fn sense(now: u32, up: [f32; 3], touch: u8) -> Sense {
        Sense {
            now_ms: now,
            up,
            accel_mg: up.map(|c| (c * 1000.0) as i16),
            touch,
        }
    }

    fn tilted(h: &Heading, toward: Compass, deg: f32) -> [f32; 3] {
        // Dipping the `toward` edge leans the sky away from it.
        let d = h.dir(toward);
        let a = deg.to_radians();
        [0, 1, 2].map(|k| libm::cosf(a) * h.up[k] as f32 - libm::sinf(a) * d[k] as f32)
    }

    #[test]
    fn tilt_walks_toward_the_dipped_edge_past_the_deadzone() {
        let cfg = ControlConfig::default();
        let h = Heading::default();
        let hold = cfg.walk_hold_ms;
        let mut c = Controls::new();
        let small = tilted(&h, Compass::North, 5.0);
        assert_eq!(c.update(&sense(0, small, 0), &h, &cfg), Buttons::NONE);
        for (dir, b) in [
            (Compass::North, Buttons::UP),
            (Compass::South, Buttons::DOWN),
            (Compass::East, Buttons::RIGHT),
            (Compass::West, Buttons::LEFT),
        ] {
            let mut c = Controls::new();
            let t = tilted(&h, dir, 25.0);
            // Not at once: the tilt has to be held.
            assert_eq!(c.update(&sense(1000, t, 0), &h, &cfg), Buttons::NONE);
            assert_eq!(c.update(&sense(1000 + hold, t, 0), &h, &cfg), b, "{dir:?}");
        }
        // Past walk_max the cube is being rolled: no walking.
        let mut c = Controls::new();
        let steep = tilted(&h, Compass::East, 50.0);
        c.update(&sense(0, steep, 0), &h, &cfg);
        assert_eq!(c.update(&sense(500, steep, 0), &h, &cfg), Buttons::NONE);
        // The same physical tilt walks the same map direction on another
        // face: roll over and check again.
        let r = h.rolled_to(h.dir(Compass::South));
        let mut c = Controls::new();
        let t = tilted(&r, Compass::East, 25.0);
        c.update(&sense(0, t, 0), &r, &cfg);
        assert_eq!(c.update(&sense(hold, t, 0), &r, &cfg), Buttons::RIGHT);
    }

    #[test]
    fn taps_long_press_and_shake() {
        let cfg = ControlConfig::default();
        let h = Heading::default();
        let up = h.up_face().index() as u8;
        let side = h.side_face(Compass::South).index() as u8;
        let level = [0.0, 1.0, 0.0];
        let mut c = Controls::new();
        // Tap the up face: A on release, for pulse_ms.
        c.update(&sense(0, level, 1 << up), &h, &cfg);
        assert_eq!(c.update(&sense(100, level, 0), &h, &cfg), Buttons::A);
        assert_eq!(c.update(&sense(150, level, 0), &h, &cfg), Buttons::A);
        assert_eq!(c.update(&sense(300, level, 0), &h, &cfg), Buttons::NONE);
        // Tap a side: B.
        c.update(&sense(1000, level, 1 << side), &h, &cfg);
        assert_eq!(c.update(&sense(1100, level, 0), &h, &cfg), Buttons::B);
        // Hold the up face: Start while held, no A after.
        c.update(&sense(2000, level, 1 << up), &h, &cfg);
        assert_eq!(c.update(&sense(2700, level, 1 << up), &h, &cfg), Buttons::START);
        assert_eq!(c.update(&sense(3000, level, 0), &h, &cfg), Buttons::NONE);
        // Shake: three jolts.
        let mut got = Buttons::NONE;
        for k in 0..6u32 {
            let mut s = sense(4000 + k * 60, level, 0);
            s.accel_mg = if k % 2 == 0 { [2500, 0, 0] } else { [0, 1000, 0] };
            got |= c.update(&s, &h, &cfg);
        }
        assert!(got.contains(Buttons::SELECT));
    }
}
