//! Pig Toss as an app: the game is [`crate::pigs`]; this keeps it in play
//! across the menu and other apps, throws the pigs when the die lands, banks
//! on a tap, and keeps the clocks the faces animate by.
//!
//! A game in play is kept until someone ends it on purpose, from the End game
//! page in the menu. Pig Toss needs one to play, so arriving without one (a
//! fresh die, settings from the phone) starts one for the table as set.

use smokebomb_hal::{HalResult, HapticEffect};

use super::{Action, App, Ctx, Effect, Effects, Kind, MotionUse, Tap, Throw};
use crate::menu::{PlayMode, Settings};
use crate::pigs::{self as game, Banked, Locked, Outcome, Pigs};
use crate::screens::pig_win;
use crate::state::Mode;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PigToss {
    /// A game is in play.
    pub live: bool,
    /// The game. Only meaningful while `live`.
    pub game: Pigs,
    /// A bank that just locked in, and when.
    pub locked: Option<(Locked, u64)>,
    /// Seconds into the pigs' tumble, and how far in they were when the die
    /// landed. Runs only while the die is shaken or thrown.
    pub clock: f32,
    pub land: f32,
    /// When the die landed, so the pigs settle from there.
    pub landed_ms: u64,
}

impl Default for PigToss {
    fn default() -> Self {
        Self {
            live: false,
            game: Pigs::default(),
            locked: None,
            clock: 0.0,
            land: 0.0,
            landed_ms: 0,
        }
    }
}

impl PigToss {
    /// Start a new game for `players`, replacing any in play.
    pub fn start(&mut self, players: u8) {
        self.game = Pigs::new(players);
        self.live = true;
    }

    /// End the game in play: its scores are gone.
    pub fn end(&mut self) {
        self.live = false;
        self.game = Pigs::new(self.game.players());
    }

    pub(super) fn settings_applied(&mut self, settings: &Settings) {
        self.locked = None;
        if settings.play() == PlayMode::PigToss && !self.live {
            self.start(settings.players);
        }
    }

    /// Seconds since the winning throw's score gave way to the win screen,
    /// once it has, while the die is between throws.
    pub fn win_t(&self, now: u64, mode: &Mode) -> Option<f32> {
        let since_landing = now.saturating_sub(self.landed_ms) as f32 / 1000.0;
        (self.game.last().is_some_and(|t| t.won)
            && matches!(mode, Mode::Idle | Mode::Reveal { .. })
            && since_landing >= pig_win::AT)
            .then_some(since_landing - pig_win::AT)
    }

    /// Run the tumble while the die is shaken or thrown, and note when it
    /// lands so the pigs can settle from there.
    pub fn animate(&mut self, dt: f32, before: &Mode, after: &Mode, now: u64) {
        let flying = |m: &Mode| matches!(m, Mode::Shaking | Mode::Airborne | Mode::Settling);
        if flying(after) {
            if !flying(before) {
                self.clock = 0.0;
            }
            // Rattling in the hand is quicker than the tumble.
            self.clock += dt * if matches!(after, Mode::Shaking) { 1.7 } else { 1.0 };
        }
        if matches!(after, Mode::Reveal { .. }) && !matches!(before, Mode::Reveal { .. }) {
            self.land = self.clock;
            self.landed_ms = now;
        }
    }
}

impl App for PigToss {
    fn kind(&self) -> Kind {
        Kind {
            motion: MotionUse::Rolls,
            scored: true,
        }
    }

    /// The pigs are a game, not a signed roll: the roll chain stays as it was.
    fn throw(&self, _settings: &Settings) -> Throw {
        Throw::Own
    }

    /// Draw both pigs' poses, and a third word for whether they landed
    /// touching.
    fn landed(&mut self, cx: &mut Ctx) -> HalResult<Effects> {
        let poses = [
            game::Pose::from_random(cx.rng.next_u32()?),
            game::Pose::from_random(cx.rng.next_u32()?),
        ];
        let touching = game::smooch_from_random(cx.rng.next_u32()?);
        let throw = self.game.throw(poses, touching);
        self.locked = None;
        let effect = match throw.outcome {
            _ if throw.won => HapticEffect::MaxCelebration,
            Outcome::Smooch => HapticEffect::Dud,
            Outcome::Bust => HapticEffect::Buzz,
            Outcome::Score(_) => HapticEffect::Tick,
        };
        let mut out = Effects::new();
        let _ = out.push(Effect::Haptic(effect));
        Ok(out)
    }

    /// Only a deliberate tap on the top screen banks: short, on a die that
    /// was resting and stays put. Picking the die up to throw it puts a
    /// finger on that screen too, and mustn't pass the turn. A winning throw
    /// still counting up isn't wiped before the win screen has had its
    /// moment.
    fn tapped(&self, tap: Tap, cx: &Ctx) -> Option<Action> {
        let ok = tap.deliberate
            && tap.face == tap.up
            && matches!(cx.mode, Mode::Idle | Mode::Reveal { .. })
            && !(self.game.last().is_some_and(|t| t.won) && self.win_t(cx.now, &cx.mode).is_none());
        ok.then_some(Action::Bank)
    }

    /// Bank the turn and pass the die, or start a new game once the win
    /// screen is up.
    fn commit(&mut self, action: Action, cx: &mut Ctx) -> Effects {
        let mut out = Effects::new();
        if action != Action::Bank {
            return out;
        }
        let players = self.game.players();
        let locked = match self.game.bank() {
            Banked::Passed {
                player,
                points,
                total,
            } => Some(Locked {
                player,
                points,
                before: total - points,
                after: total,
                next: (player + 1) % players,
            }),
            Banked::Nothing | Banked::NewGame => None,
        };
        self.locked = locked.map(|l| (l, cx.now));
        if locked.is_some() {
            // A thunk as it locks.
            let _ = out.push(Effect::Haptic(HapticEffect::LandingThud));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pigs::Pose::*;

    #[test]
    fn a_game_lives_until_it_is_ended() {
        let mut p = PigToss::default();
        assert!(!p.live);
        p.start(3);
        assert!(p.live);
        p.game.throw([Back, Back], false);
        p.game.bank();
        assert_eq!(p.game.scores(), &[20, 0, 0]);
        p.end();
        assert!(!p.live);
        assert_eq!(p.game.scores(), &[0, 0, 0], "the scores go with it");
    }

    #[test]
    fn arriving_in_pig_toss_starts_a_game_only_if_there_is_none() {
        let mut p = PigToss::default();
        let settings = Settings {
            play: PlayMode::PigToss,
            players: 4,
            ..Settings::default()
        };
        p.settings_applied(&settings);
        assert!(p.live);
        assert_eq!(p.game.players(), 4);
        p.game.throw([Back, Back], false);
        p.game.bank();
        p.settings_applied(&settings);
        assert_eq!(p.game.scores()[0], 20, "the game in play is kept");
    }
}
