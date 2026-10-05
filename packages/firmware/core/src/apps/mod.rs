//! Apps: what the die is being used for (docs/APP_FRAMEWORK.md).
//!
//! The firmware is a platform that hosts one app at a time. The platform
//! owns everything the apps share: boot, the Nest, power and sleep, the roll
//! flow (shake, air, settle), gestures, the menu and rendering. An app owns
//! its rules. It says what motion means to it ([`Kind`]), what a throw draws
//! ([`Throw`]), and what to do when it lands, is shaken, is touched or ticks;
//! it answers with [`Effect`]s for the platform to carry out.
//!
//! Every app's state lives in [`Apps`], whichever is active, so a game in
//! play (Pig Toss's scores) outlives the menu and switching apps: go to Dice
//! for a roll and back, and the game is where it was.
//!
//! Like the other machines in this crate, apps are pure: they reach the
//! hardware only through [`Ctx`] (the RNG) and the effects they return.

pub mod dice;
pub mod pigs;
pub mod pot;
pub mod potato;

use heapless::Vec;
use smokebomb_hal::{Face, HapticEffect, Rng};
use smokebomb_shared::{DieKind, ModeId};

use crate::menu::Settings;
use crate::potato::Potato;
use crate::state::Mode;

pub use dice::Dice;
pub use pigs::PigToss;
pub use pot::Pot;

/// What motion means to an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionUse {
    /// A shake and throw rolls ([`App::throw`], then [`App::landed`]).
    Rolls,
    /// A shake triggers the app ([`App::shaken`]); throws don't roll.
    Shake,
}

/// The facts about an app the platform needs to host it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub motion: MotionUse,
    /// The app keeps score for the table.
    pub scored: bool,
}

/// What a throw draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Throw {
    /// Dice that the platform rolls and signs.
    Dice(DieKind, u8),
    /// Nothing signed: the app draws what it needs in [`App::landed`].
    Own,
}

/// A touch on one face, as an app sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tap {
    pub face: Face,
    /// The face pointing up.
    pub up: Face,
    /// Short, on a die that had been resting ([`crate::gesture`]).
    pub deliberate: bool,
}

/// A change to an app's state, asked for by a gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Pig Toss: bank the turn and pass the die, or start again after a win.
    Bank,
    /// Pass the Pot: one bill fewer, wrapping from 1 back to 3.
    Bills,
    /// Pass the Pot: put the last result away for the next player.
    DismissResult,
    /// Hot Potato: reset a spent die.
    Reset,
}

/// What the smoke should do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmokeOp {
    /// Build toward this much of a full cloud, as if shaken.
    Smolder(f32),
    /// A full cloud that drains over the faces, with embers.
    Burst,
    Clear,
}

/// Something an app asks the platform to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Effect {
    /// Played if haptics are on.
    Haptic(HapticEffect),
    Smoke(SmokeOp),
    /// A round started: clear the labels and any result.
    GameStarted,
    /// Put the shown result away.
    DismissResult,
}

pub type Effects = Vec<Effect, 4>;

/// What an app may see of the platform while it handles something.
pub struct Ctx<'a> {
    pub now: u64,
    /// The roll state machine's mode.
    pub mode: Mode,
    /// The setup label is up with nothing over it ([`crate::ui::Ui::label_up`]).
    pub label_up: bool,
    pub settings: &'a mut Settings,
    pub rng: &'a mut dyn Rng,
}

pub trait App {
    fn kind(&self) -> Kind;

    /// What a throw draws. Most apps roll their dice setup.
    fn throw(&self, settings: &Settings) -> Throw {
        let (die, count) = settings.active();
        Throw::Dice(die, count)
    }

    /// A throw landed and [`App::throw`] said [`Throw::Own`].
    fn landed(&mut self, _cx: &mut Ctx) -> smokebomb_hal::HalResult<Effects> {
        Ok(Effects::new())
    }

    /// A shake, for [`MotionUse::Shake`] apps, outside the menu and Nest.
    fn shaken(&mut self, _cx: &mut Ctx) -> smokebomb_hal::HalResult<Effects> {
        Ok(Effects::new())
    }

    /// What a tap does. Taps still change game state for now; brief 3 moves
    /// these to holds (docs/APP_FRAMEWORK.md, step 3).
    fn tapped(&self, _tap: Tap, _cx: &Ctx) -> Option<Action> {
        None
    }

    /// Carry out an action.
    fn commit(&mut self, _action: Action, _cx: &mut Ctx) -> Effects {
        Effects::new()
    }

    /// Every tick, menu or not.
    fn tick(&mut self, _cx: &mut Ctx) -> Effects {
        Effects::new()
    }

    /// Something is going on that keeps the screens awake.
    fn busy(&self) -> bool {
        false
    }

    /// A hold doesn't open the menu now (a lit fuse), and shows no ring.
    fn blocks_hold(&self) -> bool {
        false
    }
}

/// Every app's state.
#[derive(Default)]
pub struct Apps {
    pub dice: Dice,
    pub pot: Pot,
    pub potato: Potato,
    pub pigs: PigToss,
}

impl Apps {
    pub fn get(&self, id: ModeId) -> &dyn App {
        match id {
            ModeId::Dice => &self.dice,
            ModeId::PassThePot => &self.pot,
            ModeId::HotPotato => &self.potato,
            ModeId::PigToss => &self.pigs,
        }
    }

    pub fn get_mut(&mut self, id: ModeId) -> &mut dyn App {
        match id {
            ModeId::Dice => &mut self.dice,
            ModeId::PassThePot => &mut self.pot,
            ModeId::HotPotato => &mut self.potato,
            ModeId::PigToss => &mut self.pigs,
        }
    }

    /// The settings were saved or replaced: a round in play is dropped, and
    /// Pig Toss, which needs a game to play, gets one if it has none.
    pub fn settings_applied(&mut self, settings: &Settings) {
        self.potato = Potato::new();
        self.pigs.settings_applied(settings);
    }

    /// The menu opened: a round in play ends, and a lock-in goes away.
    pub fn menu_opened(&mut self) {
        self.potato = Potato::new();
        self.pigs.locked = None;
    }

    /// Docked: rounds end, with what that takes (the smoke clears).
    pub fn docked(&mut self) -> Effects {
        self.pigs.locked = None;
        potato::commands(self.potato.reset())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hot_potato_is_triggered_by_a_shake_and_only_pig_toss_keeps_score() {
        let apps = Apps::default();
        for id in ModeId::ALL {
            let kind = apps.get(id).kind();
            assert_eq!(kind.motion == MotionUse::Shake, id == ModeId::HotPotato, "{id:?}");
            assert_eq!(kind.scored, id == ModeId::PigToss, "{id:?}");
        }
    }
}
