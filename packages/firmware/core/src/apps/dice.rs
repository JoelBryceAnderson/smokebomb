//! Dice: roll the dice setup. The platform rolls and signs them, and works
//! out a max or a dud; the app has nothing more to add.

use super::{App, Kind, MotionUse};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dice;

impl App for Dice {
    fn kind(&self) -> Kind {
        Kind {
            motion: MotionUse::Rolls,
            scored: false,
        }
    }
}
