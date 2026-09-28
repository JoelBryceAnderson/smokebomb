//! Top-level die state machine.
//!
//! ```text
//!            Handled            Shaking            FreeFall           Impact
//!   Idle ─────────────▶ Held ───────────▶ Shaking ──────────▶ Airborne ────────▶ Settling
//!    ▲  ◀── Rest ───────┘                    │ Rest (set down)                      │ Rest
//!    │                                       ▼                                      ▼
//!    └──────────────── timeout / Handled ── Reveal ◀──────────── roll ◀────────────┘
//!
//!   Idle/Reveal ── LongPress ──▶ Menu ── LongPress (save) / MenuTimeout ──▶ Idle
//!                                 ├──── Tap: change the setting / Restart ──▶ Menu / Idle
//!                                 └──── Shaking / FreeFall (discard) ──▶ Shaking / Airborne
//!   any ── Docked(true) ──▶ Nest ── Docked(false) ──▶ Idle
//! ```
//!
//! Tips inside the menu don't pass through here: the firmware reads them
//! from the gyro and applies them to the menu's draft ([`crate::menu`]).
//!
//! The machine is pure: it consumes [`Event`]s and returns [`Command`]s for
//! the firmware to execute, which keeps it unit-testable without hardware.

use heapless::Vec;
use smokebomb_hal::HapticEffect;

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
    /// The setup menu (SIM_SPEC C3).
    Menu,
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
    /// A tap in the menu while Restart is selected.
    Restart,
    Docked(bool),
    /// No menu input for [`crate::MENU_IDLE_MS`].
    MenuTimeout,
    Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Haptic(HapticEffect),
    Roll,
    /// Start a menu draft from the current setup.
    MenuOpen,
    /// Leave the menu, saving the draft or not.
    MenuClose {
        save: bool,
    },
    /// A tap in the menu: change the selected setting.
    MenuTap,
    /// Leave the menu without saving and restart the die.
    MenuRestart,
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
                if self.mode == Menu {
                    emit(MenuClose { save: false });
                }
                Some(Nest)
            }

            // Menu: hold again to save; a throw or a timeout discards.
            (Menu, Event::LongPress) => {
                emit(MenuClose { save: true });
                emit(Haptic(HapticEffect::MenuSave));
                Some(Idle)
            }
            (Menu, Event::Tap) => {
                emit(MenuTap);
                None
            }
            (Menu, Event::Restart) => {
                emit(MenuRestart);
                Some(Idle)
            }
            (Menu, Event::MenuTimeout) => {
                emit(MenuClose { save: false });
                Some(Idle)
            }
            (Menu, Event::Motion(Motion::Shaking)) => {
                emit(MenuClose { save: false });
                emit(Haptic(HapticEffect::Tick));
                Some(Shaking)
            }
            (Menu, Event::Motion(Motion::FreeFall)) => {
                emit(MenuClose { save: false });
                Some(Airborne)
            }
            (Menu, _) => None,
            (Idle | Reveal { .. }, Event::LongPress) => {
                emit(Haptic(HapticEffect::MenuOpen));
                emit(MenuOpen);
                Some(Menu)
            }

            // Roll flow.
            (Idle | Reveal { .. }, Event::Motion(Motion::Handled)) => Some(Held),
            (Idle | Held | Reveal { .. }, Event::Motion(Motion::Shaking)) => {
                emit(Haptic(HapticEffect::Tick));
                Some(Shaking)
            }
            (Held, Event::Motion(Motion::Rest)) => Some(Idle),
            (Held | Shaking, Event::Motion(Motion::FreeFall)) => Some(Airborne),
            (Airborne, Event::Motion(Motion::Impact)) => {
                emit(Haptic(HapticEffect::LandingThud));
                Some(Settling)
            }
            // Rest after a throw, or a shake-and-place without a throw.
            (Settling | Shaking | Airborne, Event::Motion(Motion::Rest)) => {
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
    fn hold_opens_the_menu_and_hold_again_saves() {
        let mut sm = StateMachine::new();
        let cmds = feed(&mut sm, &[Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Menu);
        assert!(cmds.contains(&Command::MenuOpen));
        assert!(cmds.contains(&Command::Haptic(HapticEffect::MenuOpen)));
        let cmds = feed(&mut sm, &[Event::Motion(Motion::Handled), Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Idle);
        assert!(cmds.contains(&Command::MenuClose { save: true }));
    }

    #[test]
    fn a_tap_in_the_menu_changes_a_setting_and_stays() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress]);
        let cmds = feed(&mut sm, &[Event::Tap]);
        assert_eq!(*sm.mode(), Mode::Menu);
        assert!(cmds.contains(&Command::MenuTap));
        assert!(!cmds.contains(&Command::MenuClose { save: true }));
    }

    #[test]
    fn restart_leaves_the_menu_without_saving() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress]);
        let cmds = feed(&mut sm, &[Event::Restart]);
        assert_eq!(*sm.mode(), Mode::Idle);
        assert!(cmds.contains(&Command::MenuRestart));
        assert!(!cmds.iter().any(|c| matches!(c, Command::MenuClose { .. })));
    }

    #[test]
    fn a_throw_timeout_or_dock_discards_the_menu() {
        for (event, mode) in [
            (Event::Motion(Motion::Shaking), Mode::Shaking),
            (Event::Motion(Motion::FreeFall), Mode::Airborne),
            (Event::MenuTimeout, Mode::Idle),
            (Event::Docked(true), Mode::Nest),
        ] {
            let mut sm = StateMachine::new();
            feed(&mut sm, &[Event::LongPress]);
            let cmds = feed(&mut sm, &[event]);
            assert_eq!(*sm.mode(), mode, "{event:?}");
            assert!(cmds.contains(&Command::MenuClose { save: false }), "{event:?}");
        }
    }

    #[test]
    fn reveal_times_out() {
        let mut sm = StateMachine::new();
        sm.mode = Mode::Reveal { since_ms: 0 };
        sm.handle(Event::Tick, REVEAL_MS);
        assert_eq!(*sm.mode(), Mode::Idle);
    }
}
