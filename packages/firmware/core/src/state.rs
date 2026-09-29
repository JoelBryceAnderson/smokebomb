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
//!                                 ├──── Tap: change the setting / Power off ──▶ Menu / Off
//!   Off ── Tap ──▶ Idle (boot)
//!                                 └──── Shaking / FreeFall (discard) ──▶ Shaking / Airborne
//!   any ── Docked(true) ──▶ Nest ── Docked(false) ──▶ Idle
//!   Nest ── LongPress ──▶ Menu ── (save / timeout) ──▶ Nest, if still docked
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
    /// Powered off: dark until a tap. There is no power switch, so this is
    /// as off as the die gets.
    Off,
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
    /// A tap in the menu while Power off is selected.
    PowerOff,
    /// A docked state began or ended (the [`crate::nest::Nest`] decides:
    /// seated in the Nest, with the seating settled).
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
    /// Leave the menu without saving and power the die off.
    MenuPowerOff,
    /// A tap while off: the screens light and the die boots.
    WakeUp,
}

pub type Commands = Vec<Command, 4>;

pub struct StateMachine {
    mode: Mode,
    /// Whether throws roll. Off in a game that doesn't (Hot Potato): motion
    /// then only matters to the menu, which a shake or a throw closes
    /// without saving.
    rolls: bool,
    /// Seated in the Nest. The menu can be open while docked, and closing it
    /// goes back to the Nest.
    docked: bool,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    pub const fn new() -> Self {
        Self {
            mode: Mode::Idle,
            rolls: true,
            docked: false,
        }
    }

    pub fn set_rolls(&mut self, rolls: bool) {
        self.rolls = rolls;
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    /// Where the die rests when nothing is going on: the Nest if seated.
    fn resting(&self) -> Mode {
        if self.docked {
            Mode::Nest
        } else {
            Mode::Idle
        }
    }

    pub fn handle(&mut self, event: Event, now_ms: u64) -> Commands {
        use Command::*;
        use Mode::*;

        let mut out = Commands::new();
        let mut emit = |c: Command| {
            let _ = out.push(c);
        };

        let next = match (self.mode, event) {
            (Nest, Event::Docked(false)) => {
                self.docked = false;
                Some(Idle)
            }
            // Picked up with the menu open (or a throw under way): the die
            // is no longer docked, but what it's doing carries on.
            (_, Event::Docked(false)) => {
                self.docked = false;
                None
            }
            // Docking doesn't block settings: a hold opens the menu.
            (Nest, Event::LongPress) => {
                emit(Haptic(HapticEffect::MenuOpen));
                emit(MenuOpen);
                Some(Menu)
            }
            (Nest, _) => None,
            (Off, Event::Tap) => {
                emit(WakeUp);
                Some(Idle)
            }
            (_, Event::Docked(true)) => {
                self.docked = true;
                if self.mode == Menu {
                    emit(MenuClose { save: false });
                }
                Some(Nest)
            }

            // Menu: hold again to save; a throw or a timeout discards.
            (Menu, Event::LongPress) => {
                emit(MenuClose { save: true });
                emit(Haptic(HapticEffect::MenuSave));
                Some(self.resting())
            }
            (Menu, Event::Tap) => {
                emit(MenuTap);
                None
            }
            (Menu, Event::PowerOff) => {
                emit(MenuPowerOff);
                Some(Off)
            }
            (Menu, Event::MenuTimeout) => {
                emit(MenuClose { save: false });
                Some(self.resting())
            }
            (Menu, Event::Motion(Motion::Shaking | Motion::FreeFall)) if !self.rolls => {
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

            // A game that doesn't roll leaves motion to the game.
            (_, Event::Motion(_)) if !self.rolls => None,

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
    fn without_rolls_motion_does_nothing_outside_the_menu() {
        let mut sm = StateMachine::new();
        sm.set_rolls(false);
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
        assert!(cmds.is_empty());
        assert_eq!(*sm.mode(), Mode::Idle);
    }

    #[test]
    fn without_rolls_a_shake_closes_the_menu_and_stays_idle() {
        let mut sm = StateMachine::new();
        sm.set_rolls(false);
        feed(&mut sm, &[Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Menu);
        let cmds = feed(&mut sm, &[Event::Motion(Motion::Shaking)]);
        assert_eq!(cmds.as_slice(), &[Command::MenuClose { save: false }]);
        assert_eq!(*sm.mode(), Mode::Idle);
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
    fn power_off_leaves_the_menu_without_saving() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress]);
        let cmds = feed(&mut sm, &[Event::PowerOff]);
        assert_eq!(*sm.mode(), Mode::Off);
        assert!(cmds.contains(&Command::MenuPowerOff));
        assert!(!cmds.iter().any(|c| matches!(c, Command::MenuClose { .. })));
    }

    #[test]
    fn only_a_tap_wakes_a_powered_off_die() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress, Event::PowerOff]);
        let cmds = feed(
            &mut sm,
            &[
                Event::Motion(Motion::Handled),
                Event::Motion(Motion::Shaking),
                Event::LongPress,
                Event::Tick,
            ],
        );
        assert_eq!(*sm.mode(), Mode::Off, "motion and holds don't wake it");
        assert!(cmds.is_empty());
        let cmds = feed(&mut sm, &[Event::Tap]);
        assert_eq!(*sm.mode(), Mode::Idle);
        assert!(cmds.contains(&Command::WakeUp));
    }

    #[test]
    fn docking_a_powered_off_die_shows_the_nest() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::LongPress, Event::PowerOff, Event::Docked(true)]);
        assert_eq!(*sm.mode(), Mode::Nest);
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
    fn a_hold_in_the_nest_opens_the_menu_and_saving_goes_back_to_the_nest() {
        let mut sm = StateMachine::new();
        feed(&mut sm, &[Event::Docked(true)]);
        let cmds = feed(&mut sm, &[Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Menu);
        assert!(cmds.contains(&Command::MenuOpen));
        feed(&mut sm, &[Event::LongPress]);
        assert_eq!(*sm.mode(), Mode::Nest);
    }

    #[test]
    fn picking_the_die_up_with_the_menu_open_keeps_the_menu() {
        let mut sm = StateMachine::new();
        feed(
            &mut sm,
            &[Event::Docked(true), Event::LongPress, Event::Docked(false)],
        );
        assert_eq!(*sm.mode(), Mode::Menu);
        feed(&mut sm, &[Event::MenuTimeout]);
        assert_eq!(*sm.mode(), Mode::Idle, "no longer docked");
    }

    #[test]
    fn reveal_times_out() {
        let mut sm = StateMachine::new();
        sm.mode = Mode::Reveal { since_ms: 0 };
        sm.handle(Event::Tick, REVEAL_MS);
        assert_eq!(*sm.mode(), Mode::Idle);
    }
}
