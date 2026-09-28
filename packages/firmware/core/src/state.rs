//! Top-level die state machine.
//!
//! ```text
//!            Handled            Shaking            FreeFall           Impact
//!   Idle ─────────────▶ Held ───────────▶ Shaking ──────────▶ Airborne ────────▶ Settling
//!    ▲  ◀── Rest ───────┘                    │ Rest (set down)                      │ Rest
//!    │                                       ▼                                      ▼
//!    └──────────────── timeout / Handled ── Reveal ◀──────────── roll ◀────────────┘
//!
//!   Idle/Reveal ── LongPress ──▶ Menu ── LongPress (save) ──▶ Idle
//!   any ── Docked(true) ──▶ Nest ── Docked(false) ──▶ Idle
//! ```
//!
//! The machine is pure: it consumes [`Event`]s and returns [`Command`]s for
//! the firmware to execute, which keeps it unit-testable without hardware.

use heapless::Vec;
use smokebomb_hal::HapticEffect;
use smokebomb_shared::assets::ClipId;

use crate::menu::{MenuInput, MenuPage};
use crate::motion::Motion;

/// How long a result stays on screen before returning to idle.
pub const REVEAL_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Held,
    Shaking,
    Airborne,
    Settling,
    Reveal {
        since_ms: u64,
    },
    Menu(MenuPage),
    /// On the charging nest.
    Nest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Motion(Motion),
    /// Short touch on any face (grip-rejected).
    Tap,
    /// Touch held for [`crate::MENU_HOLD_MS`].
    LongPress,
    Docked(bool),
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PlayClip(ClipId),
    StopClip,
    Haptic(HapticEffect),
    Roll,
    MenuInput(MenuInput),
}

pub type Commands = Vec<Command, 4>;

pub struct StateMachine {
    mode: Mode,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    pub const fn new() -> Self {
        Self { mode: Mode::Idle }
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn handle(&mut self, event: Event, now_ms: u64) -> Commands {
        use Command::*;
        use Mode::*;

        let mut out = Commands::new();
        let mut emit = |c: Command| {
            let _ = out.push(c);
        };

        let next = match (self.mode, event) {
            (Nest, Event::Docked(false)) => Some(Idle),
            (Nest, _) => None,
            (_, Event::Docked(true)) => {
                emit(StopClip);
                Some(Nest)
            }

            // Menu: tap cycles the value, tip (any handling) turns the page,
            // long-press saves and exits.
            (Menu(page), Event::Tap) => {
                emit(MenuInput(crate::menu::MenuInput::Cycle(page)));
                None
            }
            (Menu(page), Event::Motion(Motion::Handled)) => Some(Menu(page.next())),
            (Menu(_), Event::LongPress) => {
                emit(Haptic(HapticEffect::Buzz));
                Some(Idle)
            }
            (Menu(_), _) => None,
            (Idle | Reveal { .. }, Event::LongPress) => {
                emit(StopClip);
                emit(Haptic(HapticEffect::Buzz));
                Some(Menu(MenuPage::first()))
            }

            // Roll flow.
            (Idle | Reveal { .. }, Event::Motion(Motion::Handled)) => {
                emit(PlayClip(ClipId::SmokeIdle));
                Some(Held)
            }
            (Idle | Held | Reveal { .. }, Event::Motion(Motion::Shaking)) => {
                emit(PlayClip(ClipId::SmokeShake));
                emit(Haptic(HapticEffect::Tick));
                Some(Shaking)
            }
            (Held, Event::Motion(Motion::Rest)) => {
                emit(StopClip);
                Some(Idle)
            }
            (Held | Shaking, Event::Motion(Motion::FreeFall)) => {
                emit(PlayClip(ClipId::SmokeThrow));
                Some(Airborne)
            }
            (Airborne, Event::Motion(Motion::Impact)) => {
                emit(Haptic(HapticEffect::LandingThud));
                Some(Settling)
            }
            // Rest after a throw, or a shake-and-place without a throw.
            (Settling | Shaking | Airborne, Event::Motion(Motion::Rest)) => {
                emit(StopClip);
                emit(Roll);
                Some(Reveal { since_ms: now_ms })
            }
            (Reveal { since_ms }, Event::Tick) if now_ms - since_ms >= REVEAL_MS => Some(Idle),

            _ => None,
        };

        if let Some(mode) = next {
            self.mode = mode;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(sm: &mut StateMachine, events: &[Event]) -> Vec<Command, 16> {
        let mut all = Vec::new();
        for (t, e) in events.iter().enumerate() {
            for c in sm.handle(*e, t as u64) {
                all.push(c).unwrap();
            }
        }
        all
    }

    #[test]
    fn throw_produces_roll() {
        let mut sm = StateMachine::new();
        let cmds = feed(
            &mut sm,
            &[
                Event::Motion(Motion::Handled),
                Event::Motion(Motion::Shaking),
                Event::Motion(Motion::FreeFall),
                Event::Motion(Motion::Impact),
                Event::Motion(Motion::Rest),
            ],
        );
        assert!(cmds.contains(&Command::Roll));
        assert!(matches!(sm.mode(), Mode::Reveal { .. }));
    }

    #[test]
    fn pick_up_and_put_down_does_not_roll() {
        let mut sm = StateMachine::new();
        let cmds = feed(
            &mut sm,
            &[Event::Motion(Motion::Handled), Event::Motion(Motion::Rest)],
        );
        assert!(!cmds.contains(&Command::Roll));
        assert_eq!(*sm.mode(), Mode::Idle);
    }

    #[test]
    fn long_press_toggles_menu() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress]);
        assert!(matches!(sm.mode(), Mode::Menu(_)));
        feed(&mut sm, &[Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Idle);
    }

    #[test]
    fn reveal_times_out() {
        let mut sm = StateMachine::new();
        sm.mode = Mode::Reveal { since_ms: 0 };
        sm.handle(Event::Tick, REVEAL_MS);
        assert_eq!(*sm.mode(), Mode::Idle);
    }
}
