//! Pass the Pot: roll one die per bill in hand (up to three). The bills are
//! the `pot_count` setting, which the Bills in hand menu page also sets.
//!
//! A tap between rolls puts the last result away for the next player and
//! shows the bills screen; a hold while that's up opens Bills alone to change
//! the count (tip up or down, hold to save). A hold on a result opens the
//! menu as usual.

use core::fmt::Write;

use heapless::String;

use super::{Action, App, Effect, Effects, Kind, MotionUse, Pending, Tap, View};
use crate::menu::Page;
use crate::state::Mode;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pot;

impl App for Pot {
    fn kind(&self) -> Kind {
        Kind {
            motion: MotionUse::Rolls,
            scored: false,
        }
    }

    fn pending(&self, v: &View) -> Option<Pending> {
        if !v.label_up {
            return None;
        }
        let mut value = String::new();
        let _ = write!(value, "{}", v.settings.pot_count);
        Some(Pending {
            action: Action::Open(Page::Pot),
            word: "bills",
            value,
            hint: "hold: edit",
        })
    }

    /// Only deliberate taps count, so picking the die up leaves the result.
    fn tapped(&self, tap: Tap, v: &View) -> Effects {
        let mut out = Effects::new();
        if tap.deliberate && !v.label_up && matches!(v.mode, Mode::Idle | Mode::Reveal { .. }) {
            let _ = out.push(Effect::DismissResult);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Settings;
    use smokebomb_hal::Face;

    fn view(settings: &Settings, label_up: bool) -> View<'_> {
        View {
            now: 0,
            mode: Mode::Idle,
            label_up,
            settings,
        }
    }

    const TAP: Tap = Tap {
        face: Face::PosZ,
        up: Face::PosZ,
        deliberate: true,
    };

    #[test]
    fn a_tap_shows_the_bills_and_a_hold_there_adjusts_them() {
        let s = Settings::default();
        assert_eq!(
            Pot.tapped(TAP, &view(&s, false)).as_slice(),
            &[Effect::DismissResult]
        );
        assert_eq!(Pot.pending(&view(&s, false)), None, "a result: the menu");
        let p = Pot.pending(&view(&s, true)).unwrap();
        assert_eq!(p.action, Action::Open(Page::Pot));
        assert_eq!(p.value.as_str(), "3");
        assert!(Pot.tapped(TAP, &view(&s, true)).is_empty());
    }
}
