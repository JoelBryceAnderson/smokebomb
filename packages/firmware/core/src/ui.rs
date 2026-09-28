//! What each face shows, and when (SIM_SPEC C: order of precedence).
//!
//! Per face, highest first:
//! 1. boot (power-on only), unless docked, in the menu or mid-throw
//! 2. docked (Nest) screen
//! 3. menu
//! 4. result, on every face except the one facing down
//! 5. wake label, on every face except the one facing down (decision H2)
//!
//! then the (placeholder) smoke clip on top. Restart blackout, landing flash,
//! menu fade, hold ring and the success screen arrive with the menu and throw
//! work.

use smokebomb_hal::Face;

use crate::screens::BOOT_DURATION;
use crate::state::Mode;

const BOOT_MS: u64 = (BOOT_DURATION * 1000.0) as u64;
/// Wake label after boot and after setup changes (C2).
pub const WAKE_AFTER_BOOT_MS: u64 = 2_200;
/// Wake label after a tap (C2).
pub const WAKE_AFTER_TAP_MS: u64 = 3_000;
/// Result fade-in, dim delay and dim fade (C6).
const RESULT_FADE_IN_MS: f32 = 900.0;
const RESULT_DIM_AFTER_MS: u64 = 7_000;
const RESULT_DIM_FADE_MS: f32 = 1_200.0;
/// A touch on a dimmed result brings it back for this long.
const RESULT_RESTORE_MS: u64 = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Boot {
    start: u64,
    top: Face,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Ui {
    started: bool,
    boot: Option<Boot>,
    /// (start, until) of the wake label.
    wake: Option<(u64, u64)>,
    /// (reveal, dim) times of the shown result.
    result: Option<(u64, u64)>,
}

/// What to draw on one face this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FaceContent {
    Blank,
    /// Seconds since boot, and whether this face was on top at boot.
    Boot {
        t: f32,
        top: bool,
    },
    Nest,
    Menu,
    Result {
        alpha: f32,
    },
    Wake {
        alpha: f32,
    },
}

fn tumbling(mode: &Mode) -> bool {
    matches!(mode, Mode::Airborne | Mode::Settling)
}

impl Ui {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn booting(&self) -> bool {
        self.boot.is_some()
    }

    /// Advance timers. Call once per tick after the state machine ran, with
    /// the mode before and after this tick's events.
    pub fn tick(&mut self, now: u64, before: &Mode, after: &Mode, up: Face, docked: bool) {
        if !self.started {
            self.started = true;
            self.boot = Some(Boot { start: now, top: up });
        }

        let entered = |m: fn(&Mode) -> bool| !m(before) && m(after);
        // A throw starts: the result, the wake label and the boot give way.
        if entered(|m| matches!(m, Mode::Shaking)) {
            self.wake = None;
            self.result = None;
        }
        if entered(|m| matches!(m, Mode::Menu(_))) || entered(|m| matches!(m, Mode::Nest)) {
            self.wake = None;
            self.result = None;
        }
        if let Mode::Reveal { since_ms } = *after {
            if !matches!(before, Mode::Reveal { .. }) {
                self.result = Some((since_ms, since_ms + RESULT_DIM_AFTER_MS));
            }
        }

        if let Some(b) = self.boot {
            let interrupted = tumbling(after) || matches!(after, Mode::Menu(_) | Mode::Reveal { .. });
            if interrupted {
                self.boot = None;
            } else if now - b.start >= BOOT_MS {
                self.boot = None;
                if !docked {
                    self.wake(now, WAKE_AFTER_BOOT_MS);
                }
            }
        }
    }

    /// Show the setup label for `ms`, extending one already showing.
    pub fn wake(&mut self, now: u64, ms: u64) {
        let start = match self.wake {
            Some((start, until)) if now < until => start,
            _ => now,
        };
        self.wake = Some((start, now + ms));
    }

    /// A tap on a screen (C2): show the setup, and bring back a dimmed result.
    pub fn tap(&mut self, now: u64, mode: &Mode) {
        if matches!(mode, Mode::Menu(_) | Mode::Nest) {
            return;
        }
        self.touch(now);
        self.wake(now, WAKE_AFTER_TAP_MS);
    }

    /// Any touch brings back a dimmed result for a while (C6).
    pub fn touch(&mut self, now: u64) {
        if let Some((reveal, dim)) = self.result {
            if now > dim {
                self.result = Some((reveal, now + RESULT_RESTORE_MS));
            }
        }
    }

    fn result_alpha(&self, now: u64) -> f32 {
        let Some((reveal, dim)) = self.result else {
            return 0.0;
        };
        if now < reveal {
            return 0.0;
        }
        let fade_in = ((now - reveal) as f32 / RESULT_FADE_IN_MS).min(1.0);
        let dimming = if now < dim {
            1.0
        } else {
            (1.0 - (now - dim) as f32 / RESULT_DIM_FADE_MS).max(0.0)
        };
        fade_in * dimming
    }

    fn wake_alpha(&self, now: u64) -> f32 {
        match self.wake {
            Some((start, until)) if now < until => {
                let a = ((now - start) as f32 / 350.0)
                    .min((until - now) as f32 / 600.0)
                    .min(1.0);
                a * 0.85
            }
            _ => 0.0,
        }
    }

    /// What `face` shows at `now`.
    pub fn content(&self, now: u64, face: Face, up: Face, mode: &Mode, has_result: bool) -> FaceContent {
        let down = face == up.opposite();
        if let Some(b) = self.boot {
            if !matches!(mode, Mode::Nest | Mode::Menu(_)) && !tumbling(mode) {
                return FaceContent::Boot {
                    t: (now - b.start) as f32 / 1000.0,
                    top: face == b.top,
                };
            }
        }
        match mode {
            Mode::Nest => return FaceContent::Nest,
            Mode::Menu(_) => return FaceContent::Menu,
            _ => {}
        }
        let result = self.result_alpha(now);
        if has_result && result > 0.0 && !down {
            return FaceContent::Result { alpha: result };
        }
        let wake = self.wake_alpha(now);
        if wake > 0.0 && !tumbling(mode) && !down {
            return FaceContent::Wake { alpha: wake };
        }
        FaceContent::Blank
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: Face = Face::PosY;

    #[test]
    fn boot_runs_then_shows_the_wake_label() {
        let mut ui = Ui::new();
        ui.tick(0, &Mode::Idle, &Mode::Idle, UP, false);
        assert!(matches!(
            ui.content(100, Face::PosX, UP, &Mode::Idle, false),
            FaceContent::Boot { .. }
        ));
        ui.tick(6_200, &Mode::Idle, &Mode::Idle, UP, false);
        assert!(!ui.booting());
        let FaceContent::Wake { alpha } = ui.content(6_550, Face::PosX, UP, &Mode::Idle, false) else {
            panic!("wake label after boot");
        };
        assert!((alpha - 0.85).abs() < 1e-3);
        // Never on the face-down screen (H2).
        assert_eq!(
            ui.content(6_550, Face::NegY, UP, &Mode::Idle, false),
            FaceContent::Blank
        );
        // Gone after 2.2 s.
        assert_eq!(
            ui.content(8_400, Face::PosX, UP, &Mode::Idle, false),
            FaceContent::Blank
        );
    }

    #[test]
    fn a_throw_interrupts_boot_without_a_wake_label() {
        let mut ui = Ui::new();
        ui.tick(0, &Mode::Idle, &Mode::Idle, UP, false);
        ui.tick(1_000, &Mode::Shaking, &Mode::Airborne, UP, false);
        assert!(!ui.booting());
        assert_eq!(
            ui.content(1_100, Face::PosX, UP, &Mode::Airborne, false),
            FaceContent::Blank
        );
    }

    #[test]
    fn result_fades_in_dims_and_comes_back_on_touch() {
        let mut ui = Ui::new();
        ui.tick(0, &Mode::Idle, &Mode::Idle, UP, false);
        let reveal = Mode::Reveal { since_ms: 10_000 };
        ui.tick(10_000, &Mode::Settling, &reveal, UP, false);
        let alpha = |ui: &Ui, t| match ui.content(t, Face::PosX, UP, &reveal, true) {
            FaceContent::Result { alpha } => alpha,
            _ => 0.0,
        };
        assert!((alpha(&ui, 10_450) - 0.5).abs() < 0.01);
        assert_eq!(alpha(&ui, 12_000), 1.0);
        assert_eq!(alpha(&ui, 18_300), 0.0);
        ui.touch(18_300);
        assert_eq!(alpha(&ui, 18_400), 1.0);
    }
}
