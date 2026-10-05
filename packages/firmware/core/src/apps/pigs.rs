//! Pig Toss as an app: the game is [`crate::pigs`]; this keeps it in play
//! across the menu and other apps, throws the pigs when the die lands, banks
//! on a hold, and keeps the clocks the faces animate by. Once someone has
//! won, a hold opens Next: a rematch, or back to setting up Players.
//!
//! A game in play is kept until someone ends it on purpose, from the End game
//! page in the menu. Pig Toss needs one to play, so arriving without one (a
//! fresh die, settings from the phone) starts one for the table as set.

use core::fmt::Write;

use heapless::String;
use smokebomb_hal::{HalResult, HapticEffect};

use super::{Action, App, Ctx, Effect, Effects, Kind, MotionUse, Pending, Throw, View};
use crate::menu::{Page, PlayMode, Settings};
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

    /// Bank while the turn has points, or once the win screen is up, choose
    /// what comes next. A winning throw still counting up has nothing
    /// pending: its moment isn't cut short.
    fn pending(&self, v: &View) -> Option<Pending> {
        if !self.live {
            return None;
        }
        if self.game.winner().is_some() {
            return self.win_t(v.now, &v.mode).map(|_| Pending {
                action: Action::Open(Page::Next),
                word: "next",
                value: String::new(),
                hint: "hold: next",
            });
        }
        if self.game.turn() == 0 {
            return None;
        }
        let total = self.game.scores()[self.game.current() as usize] + self.game.turn();
        let mut value = String::new();
        let _ = write!(value, "{total}");
        Some(Pending {
            action: Action::Bank,
            word: "bank",
            value,
            hint: "hold: bank",
        })
    }

    /// Bank the turn and pass the die.
    fn commit(&mut self, action: Action, cx: &mut Ctx) -> Effects {
        let out = Effects::new();
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

#[cfg(test)]
mod pending_tests {
    use super::*;
    use crate::pigs::Pose::*;

    fn view(settings: &Settings, now: u64) -> View<'_> {
        View {
            now,
            mode: Mode::Idle,
            label_up: false,
            settings,
        }
    }

    #[test]
    fn a_hold_banks_once_the_turn_has_points() {
        let s = Settings::default();
        let mut p = PigToss::default();
        p.start(2);
        assert_eq!(p.pending(&view(&s, 0)), None);
        p.game.throw([Back, Back], false);
        let pending = p.pending(&view(&s, 0)).unwrap();
        assert_eq!(pending.action, Action::Bank);
        assert_eq!(pending.value.as_str(), "20");
    }

    #[test]
    fn after_a_win_a_hold_chooses_what_is_next_once_the_win_screen_is_up() {
        let s = Settings::default();
        let mut p = PigToss::default();
        p.start(2);
        while p.game.winner().is_none() {
            p.game.throw([Ear, Ear], false);
        }
        p.landed_ms = 1_000;
        assert_eq!(p.pending(&view(&s, 1_000)), None, "still counting up");
        let later = 1_000 + (pig_win::AT * 1000.0) as u64 + 100;
        assert_eq!(
            p.pending(&view(&s, later)).unwrap().action,
            Action::Open(Page::Next)
        );
    }
}
