//! Pass the Pot: roll one die per bill in hand (up to three). The bills are
//! the `pot_count` setting, which the Bills in hand menu page also sets.

use smokebomb_hal::HapticEffect;
use smokebomb_shared::types::MAX_POT_DICE;

use super::{Action, App, Ctx, Effect, Effects, Kind, MotionUse, Tap};
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

    /// A deliberate tap between rolls. The first puts the last result away
    /// and shows the bills screen; a tap while that's up takes a bill away.
    fn tapped(&self, tap: Tap, cx: &Ctx) -> Option<Action> {
        if !tap.deliberate || !matches!(cx.mode, Mode::Idle | Mode::Reveal { .. }) {
            return None;
        }
        Some(if cx.label_up {
            Action::Bills
        } else {
            Action::DismissResult
        })
    }

    fn commit(&mut self, action: Action, cx: &mut Ctx) -> Effects {
        let mut out = Effects::new();
        match action {
            Action::Bills => {
                let n = cx.settings.pot_count;
                cx.settings.pot_count = if n <= 1 { MAX_POT_DICE as u8 } else { n - 1 };
                let _ = out.push(Effect::Haptic(HapticEffect::Tick));
            }
            Action::DismissResult => {
                let _ = out.push(Effect::DismissResult);
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Settings;
    use smokebomb_hal::{Face, HalResult, Rng};

    struct NoRng;
    impl Rng for NoRng {
        fn fill_bytes(&mut self, _: &mut [u8]) -> HalResult<()> {
            unreachable!()
        }
    }

    #[test]
    fn bills_count_down_and_wrap() {
        let mut settings = Settings::default();
        let mut pot = Pot;
        let mut cx = Ctx {
            now: 0,
            mode: Mode::Idle,
            label_up: true,
            settings: &mut settings,
            rng: &mut NoRng,
        };
        let tap = Tap {
            face: Face::PosZ,
            up: Face::PosZ,
            deliberate: true,
        };
        let mut seen = std::vec::Vec::new();
        for _ in 0..3 {
            let a = pot.tapped(tap, &cx).unwrap();
            pot.commit(a, &mut cx);
            seen.push(cx.settings.pot_count);
        }
        assert_eq!(seen, [2, 1, 3]);
    }
}
