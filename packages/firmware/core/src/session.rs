//! Game sessions: a game in play, kept until someone ends it.
//!
//! A mode with a game worth keeping (Pig Toss's players and scores) holds it
//! here. A session outlives the menu and switching modes: go to Dice for a
//! roll and back, and the game is where it was. It ends only when ended on
//! purpose, from the game's End game page in the menu.
//!
//! Sugar Rush keeps its board and level the same way: open the menu or go
//! roll some dice, and the puzzle is as you left it.
//!
//! Hot Potato's rounds are seconds long and Dice and Pass the Pot keep no
//! score, so they have no session.

use smokebomb_shared::{ModeId, ModeSet};

use crate::pigs::Pigs;
use crate::rush::Rush;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sessions {
    /// The modes with a game in play.
    live: ModeSet,
    /// Pig Toss's game. Only meaningful while its session is live.
    pub pigs: Pigs,
    /// Sugar Rush's puzzle. Only meaningful while its session is live.
    pub rush: Rush,
}

impl Default for Sessions {
    fn default() -> Self {
        Self {
            live: ModeSet::EMPTY,
            pigs: Pigs::default(),
            rush: Rush::new(3, 1, 1),
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

    /// Start Sugar Rush at `level` on an `n`×`n` board, replacing any in
    /// play.
    pub fn start_rush(&mut self, n: u8, level: u16, seed: u32) {
        self.rush = Rush::new(n, level, seed);
        self.live = self.live.with(ModeId::SugarRush);
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
