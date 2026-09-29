//! What each face shows, and when (SIM_SPEC C: order of precedence).
//!
//! Per face, highest first:
//! 1. boot (power-on only), unless docked, in the menu or mid-throw
//! 2. docked (Nest) screen
//! 3. menu
//! 4. result, on every face except the one facing down
//! 5. wake label, on every face except the one facing down (decision H2)
//!
//! with the success screen after a saved menu between the result and the
//! wake label, and then the (placeholder) smoke clip on top. Under the
//! content go the save flash, the fading menu and the hold ring; a restart
//! blacks everything out. The landing flash arrives with the throw work.

use smokebomb_hal::Face;

use crate::menu::{Draft, Setup};
use crate::screens::{BOOT_DURATION, BURST_AT, LOOP_END};
use crate::smoke::Special;
use crate::state::Mode;

const BOOT_MS: u64 = (BOOT_DURATION * 1000.0) as u64;
/// The top face's centre pip bursts into smoke this long into the boot.
const BURST_MS: u64 = ((LOOP_END + BURST_AT) * 1000.0 + 0.5) as u64;
/// A max or dud keeps the result lit at least this long (SIM_SPEC C6).
const SPECIAL_LIT_MS: u64 = 5_000;
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
/// Menu grow-in (C3): alpha over 0.3 s, scale and ring flash over 0.35 s.
const MENU_FADE_IN_MS: f32 = 300.0;
const MENU_GROW_MS: f32 = 350.0;
/// Menu fade-out after it closes, and the save flash (C4).
const MENU_FADE_OUT_MS: f32 = 300.0;
const FLASH_MS: f32 = 250.0;
/// The success screen on the face the menu was on (C4).
pub const SUCCESS_MS: u64 = 1_300;
/// Screens stay dark this long before a restart boots (C3).
pub const RESTART_BLACKOUT_MS: u64 = 800;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Boot {
    start: u64,
    top: Face,
    burst: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Ui {
    started: bool,
    boot: Option<Boot>,
    /// (start, until) of the wake label.
    wake: Option<(u64, u64)>,
    /// (reveal, dim) times of the shown result.
    result: Option<(u64, u64)>,
    /// A max or dud, once its effect has started.
    special: Option<Special>,
    menu_open_at: Option<u64>,
    /// The menu fading out: when it closed, what it showed, and where.
    menu_fade: Option<(u64, Draft, Face)>,
    /// A saved setup: when, on which face, and what.
    success: Option<(u64, Face, Setup)>,
    flash: Option<u64>,
    /// Restarting: dark until then, then boot.
    blackout_until: Option<u64>,
}

/// How the menu enters (C3): its alpha and scale, and the hold ring that
/// filled on the way in flashing outward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuIntro {
    pub alpha: f32,
    pub scale: f32,
    pub ring_alpha: f32,
    pub ring_grow: f32,
}

/// The mockup's `ease`: ease-out cubic.
pub fn ease(x: f32) -> f32 {
    let u = 1.0 - x.clamp(0.0, 1.0);
    1.0 - u * u * u
}

fn since(now: u64, t: u64) -> f32 {
    now.saturating_sub(t) as f32
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
    /// Seconds since a setup was saved, and the setup.
    Success {
        t: f32,
        setup: Setup,
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
        self.boot.is_some() || self.blackout_until.is_some()
    }

    /// The boot's smoke burst is due: the top face, once per boot.
    pub fn take_boot_burst(&mut self, now: u64) -> Option<Face> {
        let b = self.boot.as_mut()?;
        if b.burst || now.saturating_sub(b.start) < BURST_MS {
            return None;
        }
        b.burst = true;
        Some(b.top)
    }

    /// A roll's result exists, from its reveal until something clears it (a
    /// shake, the menu, docking, a restart), dimmed or not.
    pub fn has_result(&self) -> bool {
        self.result.is_some()
    }

    /// A result is up (revealed, possibly dimmed).
    pub fn showing_result(&self, now: u64) -> bool {
        self.result.is_some_and(|(reveal, _)| now >= reveal)
    }

    /// A max or dud's effect started: the result says so and stays lit.
    pub fn set_special(&mut self, now: u64, special: Special) {
        self.special = Some(special);
        if let Some((reveal, dim)) = self.result {
            self.result = Some((reveal, dim.max(now + SPECIAL_LIT_MS)));
        }
    }

    pub fn special(&self) -> Option<Special> {
        self.special
    }

    /// Restarting: every screen is dark.
    pub fn blackout(&self) -> bool {
        self.blackout_until.is_some()
    }

    pub fn menu_opened(&mut self, now: u64) {
        self.menu_open_at = Some(now);
        self.menu_fade = None;
    }

    /// The menu closed on `face` showing `draft`. Saving shows the success
    /// screen there and flashes; either way the menu fades and the setup
    /// label shows (C4).
    pub fn menu_closed(&mut self, now: u64, face: Face, draft: Draft, saved: bool) {
        self.menu_open_at = None;
        self.menu_fade = Some((now, draft, face));
        if saved {
            self.success = Some((now, face, draft.setup()));
            self.flash = Some(now);
        }
        self.wake(now, WAKE_AFTER_BOOT_MS);
    }

    /// A game starts (Hot Potato lit): the boot and the labels give way.
    pub fn game_started(&mut self) {
        self.boot = None;
        self.wake = None;
        self.result = None;
        self.success = None;
    }

    /// Holding on Restart: dark for a moment, then boot again (C3, C1).
    pub fn restart(&mut self, now: u64) {
        *self = Self {
            started: true,
            blackout_until: Some(now + RESTART_BLACKOUT_MS),
            ..Self::default()
        };
    }

    pub fn menu_intro(&self, now: u64) -> MenuIntro {
        let t = self.menu_open_at.map_or(f32::MAX, |t| since(now, t));
        let e = (t / MENU_GROW_MS).min(1.0);
        MenuIntro {
            alpha: (t / MENU_FADE_IN_MS).min(1.0),
            scale: 0.86 + 0.14 * ease(e),
            ring_alpha: 1.0 - e,
            ring_grow: e * 3.0,
        }
    }

    /// The closed menu fading out on its face: the draft, and progress 0–1.
    pub fn menu_fade(&self, now: u64, face: Face) -> Option<(Draft, f32)> {
        let (t, draft, f) = self.menu_fade?;
        let u = since(now, t) / MENU_FADE_OUT_MS;
        (f == face && u < 1.0).then_some((draft, u))
    }

    /// The save flash's alpha (before its 35%).
    pub fn flash(&self, now: u64) -> f32 {
        self.flash
            .map_or(0.0, |t| (1.0 - since(now, t) / FLASH_MS).max(0.0))
    }

    /// Advance timers. Call once per tick after the state machine ran, with
    /// the mode before and after this tick's events.
    pub fn tick(&mut self, now: u64, before: &Mode, after: &Mode, up: Face, docked: bool) {
        if !self.started {
            self.started = true;
            self.boot = Some(Boot {
                start: now,
                top: up,
                burst: false,
            });
        }
        if let Some(until) = self.blackout_until {
            if now >= until {
                self.blackout_until = None;
                self.boot = Some(Boot {
                    start: now,
                    top: up,
                    burst: false,
                });
            }
        }

        let entered = |m: fn(&Mode) -> bool| !m(before) && m(after);
        // A throw starts: the result, the wake label and the boot give way.
        if entered(|m| matches!(m, Mode::Shaking)) {
            self.wake = None;
            self.result = None;
            self.special = None;
        }
        if entered(|m| matches!(m, Mode::Menu)) || entered(|m| matches!(m, Mode::Nest)) {
            self.wake = None;
            self.result = None;
            self.special = None;
            self.success = None;
        }
        if let Mode::Reveal { since_ms } = *after {
            if !matches!(before, Mode::Reveal { .. }) {
                self.result = Some((since_ms, since_ms + RESULT_DIM_AFTER_MS));
                self.special = None;
            }
        }

        if let Some(b) = self.boot {
            let interrupted = tumbling(after) || matches!(after, Mode::Menu | Mode::Reveal { .. });
            if interrupted {
                self.boot = None;
            } else if now.saturating_sub(b.start) >= BOOT_MS {
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
        if matches!(mode, Mode::Menu | Mode::Nest) || self.blackout() {
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
        let fade_in = ((now.saturating_sub(reveal)) as f32 / RESULT_FADE_IN_MS).min(1.0);
        let dimming = if now < dim {
            1.0
        } else {
            (1.0 - (now.saturating_sub(dim)) as f32 / RESULT_DIM_FADE_MS).max(0.0)
        };
        fade_in * dimming
    }

    fn wake_alpha(&self, now: u64) -> f32 {
        match self.wake {
            Some((start, until)) if now < until => {
                let a = ((now.saturating_sub(start)) as f32 / 350.0)
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
        if self.blackout() {
            return FaceContent::Blank;
        }
        if let Some(b) = self.boot {
            if !matches!(mode, Mode::Nest | Mode::Menu) && !tumbling(mode) {
                return FaceContent::Boot {
                    t: (now.saturating_sub(b.start)) as f32 / 1000.0,
                    top: face == b.top,
                };
            }
        }
        match mode {
            Mode::Nest => return FaceContent::Nest,
            Mode::Menu => return FaceContent::Menu,
            _ => {}
        }
        let result = self.result_alpha(now);
        if has_result && result > 0.0 && !down {
            return FaceContent::Result { alpha: result };
        }
        if let Some((t, f, setup)) = self.success {
            if f == face && now.saturating_sub(t) < SUCCESS_MS && !tumbling(mode) {
                let t = since(now, t) / 1000.0;
                return FaceContent::Success { t, setup };
            }
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
    fn saving_the_menu_shows_success_then_the_wake_label() {
        let mut ui = Ui::new();
        ui.tick(0, &Mode::Idle, &Mode::Idle, UP, false);
        ui.tick(9_000, &Mode::Idle, &Mode::Menu, UP, false);
        ui.menu_opened(9_000);
        assert!(!ui.booting(), "the menu ends the boot");
        let intro = ui.menu_intro(9_150);
        assert!((intro.alpha - 0.5).abs() < 1e-3);
        let draft = Draft::new(&crate::menu::Settings::default());
        ui.menu_closed(12_000, Face::PosZ, draft, true);
        ui.tick(12_000, &Mode::Menu, &Mode::Idle, UP, false);
        assert!(ui.menu_fade(12_150, Face::PosZ).is_some());
        assert!(ui.menu_fade(12_150, Face::PosX).is_none());
        assert!(ui.flash(12_100) > 0.0);
        assert!(matches!(
            ui.content(12_500, Face::PosZ, UP, &Mode::Idle, false),
            FaceContent::Success { .. }
        ));
        assert!(matches!(
            ui.content(12_500, Face::PosX, UP, &Mode::Idle, false),
            FaceContent::Wake { .. }
        ));
        assert!(matches!(
            ui.content(13_500, Face::PosZ, UP, &Mode::Idle, false),
            FaceContent::Wake { .. }
        ));
    }

    #[test]
    fn restart_blacks_out_then_boots() {
        let mut ui = Ui::new();
        ui.tick(0, &Mode::Idle, &Mode::Idle, UP, false);
        ui.tick(7_000, &Mode::Idle, &Mode::Idle, UP, false);
        ui.restart(9_000);
        ui.tick(9_000, &Mode::Menu, &Mode::Idle, UP, false);
        assert_eq!(
            ui.content(9_500, Face::PosX, UP, &Mode::Idle, false),
            FaceContent::Blank
        );
        ui.tick(9_800, &Mode::Idle, &Mode::Idle, UP, false);
        assert!(matches!(
            ui.content(9_900, Face::PosX, UP, &Mode::Idle, false),
            FaceContent::Boot { .. }
        ));
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
