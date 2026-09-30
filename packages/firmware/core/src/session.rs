//! Game sessions: a game in play, kept until someone ends it.
//!
//! A mode with a game worth keeping (Pig Toss's players and scores) holds it
//! here. A session outlives the menu and switching modes: go to Dice for a
//! roll and back, and the game is where it was. It ends only when ended on
//! purpose, from the game's End game page in the menu.
//!
//! Hot Potato's rounds are seconds long and Dice and Pass the Pot keep no
//! score, so for now Pig Toss is the only mode with a session.

use smokebomb_shared::{ModeId, ModeSet};

use crate::pigs::Pigs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sessions {
    /// The modes with a game in play.
    live: ModeSet,
    /// Pig Toss's game. Only meaningful while its session is live.
    pub pigs: Pigs,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            live: ModeSet::EMPTY,
            pigs: Pigs::default(),
        }
    }
}

impl Sessions {
    /// Whether `mode` has a game in play.
    pub fn live(&self, mode: ModeId) -> bool {
        self.live.contains(mode)
    }

    /// Start a new Pig Toss game for `players`, replacing any in play.
    pub fn start_pigs(&mut self, players: u8) {
        self.pigs = Pigs::new(players);
        self.live = self.live.with(ModeId::PigToss);
    }

    /// End `mode`'s game: its state is gone.
    pub fn end(&mut self, mode: ModeId) {
        self.live = self.live.without(mode);
        if mode == ModeId::PigToss {
            self.pigs = Pigs::new(self.pigs.players());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pigs::Pose::*;

    #[test]
    fn a_session_lives_until_it_is_ended() {
        let mut s = Sessions::default();
        assert!(!s.live(ModeId::PigToss));
        s.start_pigs(3);
        assert!(s.live(ModeId::PigToss));
        assert!(!s.live(ModeId::Dice));
        s.pigs.throw([Back, Back], false);
        s.pigs.bank();
        assert_eq!(s.pigs.scores(), &[20, 0, 0]);
        s.end(ModeId::PigToss);
        assert!(!s.live(ModeId::PigToss));
        assert_eq!(s.pigs.scores(), &[0, 0, 0], "the scores go with it");
    }
}
