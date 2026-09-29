//! Pig Toss.
//!
//! Two pigs are thrown every time. Each lands in one of six poses, drawn at
//! the odds a tossed pig-shaped token really lands in, so a pose is worth
//! more the rarer it is. Throw as often as you like to build up the turn's
//! score, then tap to bank it and pass the die; a bust loses the turn's
//! score.
//!
//! | Pose | Odds | Points |
//! |---|---|---|
//! | Side (dot up / plain up) | 30.2 % / 34.9 % | 0 |
//! | Back | 22.4 % | 5 |
//! | Feet | 8.8 % | 5 |
//! | Nose | 3.0 % | 10 |
//! | Ear | 0.7 % | 15 |
//!
//! - Two sides that don't match (one dot up, one plain up) is a **bust**:
//!   the turn's score is lost and the die passes.
//! - Two matching sides score 1 point.
//! - A side next to any other pose scores only the other pig.
//! - Two matching poses that aren't sides is a **double**: four times one
//!   pig, so a double back is 20 and a double ear is 60.
//! - Anything else is the two pigs' points added together.
//!
//! First to [`TARGET`] wins, once they bank it.
//!
//! Like [`crate::potato::Potato`] this is pure: the firmware draws the
//! poses from its RNG and passes them in.

use core::fmt::Write;

use heapless::String;

pub const MIN_PLAYERS: u8 = 2;
pub const MAX_PLAYERS: u8 = 6;
/// Banked points that win the game.
pub const TARGET: u16 = 100;

/// How a pig can land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pose {
    /// On its side, the dot up.
    SideDot,
    /// On its side, the bare side up.
    SidePlain,
    /// On its back.
    Back,
    /// Standing on its feet.
    Feet,
    /// Balanced on its nose and front feet.
    Nose,
    /// Leaning on its nose, a front foot and an ear.
    Ear,
}

/// Weights out of [`WEIGHT_TOTAL`], in the order of [`Pose::ALL`].
const WEIGHTS: [u16; 6] = [3020, 3490, 2240, 880, 300, 70];
pub const WEIGHT_TOTAL: u32 = 10_000;

impl Pose {
    pub const ALL: [Pose; 6] = [
        Pose::SideDot,
        Pose::SidePlain,
        Pose::Back,
        Pose::Feet,
        Pose::Nose,
        Pose::Ear,
    ];

    /// Chance out of [`WEIGHT_TOTAL`].
    pub fn weight(self) -> u16 {
        WEIGHTS[self.index()]
    }

    /// The pose a raw random word lands on.
    pub fn from_random(r: u32) -> Pose {
        // Scale into 0..WEIGHT_TOTAL without the bias a remainder has.
        let mut n = ((r as u64 * WEIGHT_TOTAL as u64) >> 32) as u32;
        for pose in Pose::ALL {
            let w = pose.weight() as u32;
            if n < w {
                return pose;
            }
            n -= w;
        }
        Pose::SidePlain
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn is_side(self) -> bool {
        matches!(self, Pose::SideDot | Pose::SidePlain)
    }

    /// What one pig in this pose is worth.
    pub fn points(self) -> u16 {
        match self {
            Pose::SideDot | Pose::SidePlain => 0,
            Pose::Back | Pose::Feet => 5,
            Pose::Nose => 10,
            Pose::Ear => 15,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Pose::SideDot | Pose::SidePlain => "Side",
            Pose::Back => "Back",
            Pose::Feet => "Feet",
            Pose::Nose => "Nose",
            Pose::Ear => "Ear",
        }
    }
}

/// What a throw of two pigs comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Add these to the turn.
    Score(u16),
    /// Sides that don't match: the turn's points are gone and the die passes.
    Bust,
}

/// The value of a throw of two pigs.
pub fn score(poses: [Pose; 2]) -> Outcome {
    let [a, b] = poses;
    match (a, b) {
        (Pose::SideDot, Pose::SidePlain) | (Pose::SidePlain, Pose::SideDot) => Outcome::Bust,
        _ if a.is_side() && b.is_side() => Outcome::Score(1),
        _ if a == b => Outcome::Score(a.points() * 4),
        _ => Outcome::Score(a.points() + b.points()),
    }
}

/// A name for a throw: `Double Feet`, `Back + Nose`, `Bust`.
pub fn throw_label(poses: [Pose; 2]) -> String<32> {
    let [a, b] = poses;
    let mut s = String::new();
    let _ = match score(poses) {
        Outcome::Bust => write!(s, "Bust"),
        Outcome::Score(1) if a.is_side() && b.is_side() => write!(s, "Sides"),
        _ if a == b => write!(s, "Double {}", a.name()),
        _ if a.is_side() => write!(s, "{}", b.name()),
        _ if b.is_side() => write!(s, "{}", a.name()),
        _ => write!(s, "{} + {}", a.name(), b.name()),
    };
    s
}

/// The last throw, kept for the faces and the simulator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Throw {
    pub poses: [Pose; 2],
    pub outcome: Outcome,
    /// Who threw (0-based).
    pub player: u8,
    /// The turn's points before this throw (what a bust throws away).
    pub turn_before: u16,
}

/// What banking did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banked {
    /// Nothing to bank: the tap changes nothing.
    Nothing,
    /// The turn's points went to the player, who is now on `total`, and the
    /// die passed on.
    Passed { player: u8, points: u16, total: u16 },
    /// The banked points reached the target.
    Won { player: u8, total: u16 },
    /// A tap on a finished game starts the next one.
    NewGame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pigs {
    players: u8,
    scores: [u16; MAX_PLAYERS as usize],
    current: u8,
    turn: u16,
    winner: Option<u8>,
    last: Option<Throw>,
}

impl Default for Pigs {
    fn default() -> Self {
        Self::new(MIN_PLAYERS)
    }
}

impl Pigs {
    pub fn new(players: u8) -> Self {
        Self {
            players: players.clamp(MIN_PLAYERS, MAX_PLAYERS),
            scores: [0; MAX_PLAYERS as usize],
            current: 0,
            turn: 0,
            winner: None,
            last: None,
        }
    }

    pub fn players(&self) -> u8 {
        self.players
    }

    /// Banked scores, one per player.
    pub fn scores(&self) -> &[u16] {
        &self.scores[..self.players as usize]
    }

    /// Whose turn it is (0-based).
    pub fn current(&self) -> u8 {
        self.current
    }

    /// Points thrown this turn and not yet banked.
    pub fn turn(&self) -> u16 {
        self.turn
    }

    pub fn winner(&self) -> Option<u8> {
        self.winner
    }

    pub fn last(&self) -> Option<&Throw> {
        self.last.as_ref()
    }

    /// Throws the die again. The two poses are already drawn.
    pub fn throw(&mut self, poses: [Pose; 2]) -> Throw {
        let outcome = score(poses);
        let throw = Throw {
            poses,
            outcome,
            player: self.current,
            turn_before: self.turn,
        };
        if self.winner.is_none() {
            match outcome {
                Outcome::Score(points) => self.turn += points,
                Outcome::Bust => {
                    self.turn = 0;
                    self.advance();
                }
            }
            self.last = Some(throw);
        }
        throw
    }

    /// A tap: bank the turn's points and pass the die, or start again after
    /// a win. With nothing thrown this turn it does nothing.
    pub fn bank(&mut self) -> Banked {
        if self.winner.is_some() {
            *self = Self::new(self.players);
            return Banked::NewGame;
        }
        if self.turn == 0 {
            return Banked::Nothing;
        }
        self.last = None;
        let player = self.current;
        let points = self.turn;
        let total = self.scores[player as usize] + points;
        self.scores[player as usize] = total;
        self.turn = 0;
        if total >= TARGET {
            self.winner = Some(player);
            Banked::Won { player, total }
        } else {
            self.advance();
            Banked::Passed {
                player,
                points,
                total,
            }
        }
    }

    fn advance(&mut self) {
        self.current = (self.current + 1) % self.players;
    }

    /// The line the faces show while the die is waiting: whose go it is.
    pub fn status(&self) -> String<24> {
        let mut s = String::new();
        let _ = match self.winner {
            Some(p) => write!(s, "P{} wins!", p + 1),
            None => write!(s, "P{} to roll", self.current + 1),
        };
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Pose::*;

    #[test]
    fn odds_add_up_and_follow_value() {
        assert_eq!(
            Pose::ALL.iter().map(|p| p.weight() as u32).sum::<u32>(),
            WEIGHT_TOTAL
        );
        // The rarer the pose, the more it is worth.
        assert!(Back.weight() > Feet.weight());
        assert!(Feet.weight() > Nose.weight());
        assert!(Nose.weight() > Ear.weight());
        assert!(Feet.points() <= Nose.points() && Nose.points() < Ear.points());
    }

    #[test]
    fn random_words_land_on_poses_in_proportion() {
        let mut counts = [0u32; 6];
        let n = 100_000u32;
        for i in 0..n {
            // An even spread over the whole u32 range.
            let r = ((i as u64 * (u32::MAX as u64 + 1)) / n as u64) as u32;
            counts[Pose::from_random(r).index()] += 1;
        }
        for pose in Pose::ALL {
            let want = pose.weight() as u32 * n / WEIGHT_TOTAL;
            let got = counts[pose.index()];
            assert!(got.abs_diff(want) <= 2, "{pose:?}: {got} vs {want}");
        }
        assert_eq!(Pose::from_random(0), SideDot);
        assert_eq!(Pose::from_random(u32::MAX), Ear);
    }

    #[test]
    fn scoring_follows_the_rules() {
        assert_eq!(score([SideDot, SidePlain]), Outcome::Bust);
        assert_eq!(score([SidePlain, SideDot]), Outcome::Bust);
        assert_eq!(score([SideDot, SideDot]), Outcome::Score(1));
        assert_eq!(score([SidePlain, SidePlain]), Outcome::Score(1));
        assert_eq!(score([SidePlain, Nose]), Outcome::Score(10));
        assert_eq!(score([Feet, SideDot]), Outcome::Score(5));
        assert_eq!(score([Back, Feet]), Outcome::Score(10));
        assert_eq!(score([Back, Back]), Outcome::Score(20));
        assert_eq!(score([Feet, Feet]), Outcome::Score(20));
        assert_eq!(score([Nose, Nose]), Outcome::Score(40));
        assert_eq!(score([Ear, Ear]), Outcome::Score(60));
        assert_eq!(score([Nose, Ear]), Outcome::Score(25));
    }

    #[test]
    fn scoring_is_symmetric() {
        for a in Pose::ALL {
            for b in Pose::ALL {
                assert_eq!(score([a, b]), score([b, a]));
            }
        }
    }

    #[test]
    fn labels_name_the_throw() {
        assert_eq!(throw_label([SideDot, SidePlain]).as_str(), "Bust");
        assert_eq!(throw_label([SidePlain, SidePlain]).as_str(), "Sides");
        assert_eq!(throw_label([Feet, Feet]).as_str(), "Double Feet");
        assert_eq!(throw_label([SideDot, Nose]).as_str(), "Nose");
        assert_eq!(throw_label([Back, Feet]).as_str(), "Back + Feet");
    }

    #[test]
    fn throws_build_the_turn_and_banking_passes_the_die() {
        let mut g = Pigs::new(3);
        g.throw([Back, Feet]);
        g.throw([Nose, SideDot]);
        assert_eq!(g.turn(), 20);
        assert_eq!(g.current(), 0);
        assert_eq!(
            g.bank(),
            Banked::Passed {
                player: 0,
                points: 20,
                total: 20
            }
        );
        assert_eq!((g.turn(), g.current()), (0, 1));
        assert_eq!(g.scores(), &[20, 0, 0]);
    }

    #[test]
    fn a_bust_loses_the_turn_but_not_the_bank() {
        let mut g = Pigs::new(2);
        g.throw([Back, Back]);
        g.bank();
        g.throw([Feet, Feet]);
        let t = g.throw([SideDot, SidePlain]);
        assert_eq!(t.outcome, Outcome::Bust);
        assert_eq!(g.turn(), 0);
        assert_eq!(g.current(), 0, "back round to the first player");
        assert_eq!(g.scores(), &[20, 0]);
    }

    #[test]
    fn banking_with_nothing_thrown_changes_nothing() {
        let mut g = Pigs::new(2);
        assert_eq!(g.bank(), Banked::Nothing);
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn the_turns_wrap_round_the_table() {
        let mut g = Pigs::new(3);
        for _ in 0..3 {
            g.throw([Back, Feet]);
            g.bank();
        }
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn banking_the_target_wins_and_a_tap_starts_again() {
        let mut g = Pigs::new(2);
        for _ in 0..5 {
            g.throw([Back, Back]);
        }
        assert_eq!(g.turn(), 100);
        assert_eq!(g.winner(), None, "not won until banked");
        assert_eq!(
            g.bank(),
            Banked::Won {
                player: 0,
                total: 100
            }
        );
        assert_eq!(g.winner(), Some(0));
        assert_eq!(g.status().as_str(), "P1 wins!");
        g.throw([Back, Feet]);
        assert_eq!(g.turn(), 0, "no throwing once it's won");
        assert_eq!(g.bank(), Banked::NewGame);
        assert_eq!(g.scores(), &[0, 0]);
        assert_eq!(g.winner(), None);
        assert_eq!(g.players(), 2);
    }

    #[test]
    fn the_player_count_is_kept_in_range() {
        assert_eq!(Pigs::new(0).players(), MIN_PLAYERS);
        assert_eq!(Pigs::new(40).players(), MAX_PLAYERS);
    }
}
