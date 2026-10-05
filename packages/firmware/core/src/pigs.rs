//! Pig Toss.
//!
//! Two pigs are thrown every time. Each lands in one of six poses, drawn at
//! the odds a tossed pig-shaped token really lands in, so a pose is worth
//! more the rarer it is. Throw as often as you like to build up the turn's
//! score, then hold to bank it and pass the die; an oops loses the turn's
//! score.
//!
//! | Pose | Odds | Points |
//! |---|---|---|
//! | Snooze (dot up / plain up) | 30.2 % / 34.9 % | 0 |
//! | Belly Up | 22.4 % | 5 |
//! | Strut | 8.8 % | 5 |
//! | Nose Dive | 3.0 % | 10 |
//! | Tipsy | 0.7 % | 15 |
//!
//! - Two snoozes that don't match (one dot up, one plain up) is an **oops**:
//!   the turn's score is lost and the die passes.
//! - Two matching snoozes are **nap time**: 1 point.
//! - A snooze next to any other pose scores only the other pig.
//! - Two matching poses that aren't snoozes are a **twin**: four times one
//!   pig, so twin belly up is 20 and twin tipsy is 60.
//! - Anything else is the two pigs' points added together.
//!
//! - The two pigs must land apart. If they end up **touching** (a
//!   **smooch**, about one throw in a hundred) the player's whole banked
//!   score goes back to 0, the turn is lost and the die passes.
//!
//! First to [`TARGET`] wins: the throw that takes the player's banked
//! score plus the turn's points there wins on the spot, with no bank.
//!
//! Like [`crate::potato::Potato`] this is pure: the firmware draws the
//! poses from its RNG and passes them in.

use core::fmt::Write;

use heapless::String;

pub const MIN_PLAYERS: u8 = 2;
pub const MAX_PLAYERS: u8 = 6;
/// The score that wins the game.
pub const TARGET: u16 = 100;

/// A small picture a player can take instead of an initial, like a board
/// game's playing pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Hat,
    Car,
    Boot,
    Boat,
    Crown,
    Star,
}

impl Symbol {
    pub const ALL: [Symbol; 6] = [
        Symbol::Hat,
        Symbol::Car,
        Symbol::Boot,
        Symbol::Boat,
        Symbol::Crown,
        Symbol::Star,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Symbol::Hat => "Top hat",
            Symbol::Car => "Car",
            Symbol::Boot => "Boot",
            Symbol::Boat => "Boat",
            Symbol::Crown => "Crown",
            Symbol::Star => "Star",
        }
    }
}

/// What marks a player on the faces: an initial, A to Z, or a [`Symbol`].
/// Scrolling runs through the letters and then the symbols, and wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token(u8);

impl Token {
    const LETTERS: u8 = 26;
    /// How many tokens there are to scroll through.
    pub const COUNT: u8 = Self::LETTERS + Symbol::ALL.len() as u8;

    pub const fn letter(c: char) -> Self {
        Self((c as u8).wrapping_sub(b'A') % Self::LETTERS)
    }

    pub const fn symbol(s: Symbol) -> Self {
        Self(Self::LETTERS + s as u8)
    }

    /// Player `i`'s token on a fresh die: A, B, C…
    pub const fn default_for(i: u8) -> Self {
        Self(i % Self::LETTERS)
    }

    /// The next token (`by` 1) or the previous one (`by` -1), wrapping.
    pub fn stepped(self, by: i32) -> Self {
        Self((self.0 as i32 + by).rem_euclid(Self::COUNT as i32) as u8)
    }

    /// The initial, if this token is a letter.
    pub fn initial(self) -> Option<char> {
        (self.0 < Self::LETTERS).then(|| (b'A' + self.0) as char)
    }

    /// The symbol, if this token is one.
    pub fn as_symbol(self) -> Option<Symbol> {
        Symbol::ALL
            .get(self.0.checked_sub(Self::LETTERS)? as usize)
            .copied()
    }
}

/// Every player's token on a fresh die.
pub const DEFAULT_TOKENS: [Token; MAX_PLAYERS as usize] = {
    let mut t = [Token(0); MAX_PLAYERS as usize];
    let mut i = 0;
    while i < t.len() {
        t[i] = Token::default_for(i as u8);
        i += 1;
    }
    t
};

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
/// Chance out of [`WEIGHT_TOTAL`] that the two pigs land touching.
pub const SMOOCH_ODDS: u32 = 100;

/// Whether a random word lands the pigs touching.
pub fn smooch_from_random(r: u32) -> bool {
    ((r as u64 * WEIGHT_TOTAL as u64) >> 32) < SMOOCH_ODDS as u64
}

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
            Pose::SideDot | Pose::SidePlain => "Snooze",
            Pose::Back => "Belly Up",
            Pose::Feet => "Strut",
            Pose::Nose => "Nose Dive",
            Pose::Ear => "Tipsy",
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
    /// The pigs touched: the player's whole banked score is gone, and the
    /// turn's, and the die passes.
    Smooch,
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

/// A name for a throw: `Twin Strut`, `Belly Up + Nose Dive`, `Oops`.
pub fn throw_label(poses: [Pose; 2], touching: bool) -> String<32> {
    let [a, b] = poses;
    let mut s = String::new();
    let _ = match score(poses) {
        _ if touching => write!(s, "Smooch"),
        Outcome::Bust => write!(s, "Oops"),
        Outcome::Score(1) if a.is_side() && b.is_side() => write!(s, "Nap Time"),
        _ if a == b => write!(s, "Twin {}", a.name()),
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
    /// The pigs landed touching.
    pub touching: bool,
    /// Who threw (0-based).
    pub player: u8,
    /// The turn's points before this throw (what a bust throws away).
    pub turn_before: u16,
    /// The player's banked score before this throw (what a smooch wipes).
    pub banked_before: u16,
    /// This throw took the player to the target and won the game.
    pub won: bool,
}

/// What the faces show while a bank locks in: who banked what, and who has
/// the die next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locked {
    pub player: u8,
    /// The points banked.
    pub points: u16,
    /// The player's score before and after.
    pub before: u16,
    pub after: u16,
    /// Who the die passes to.
    pub next: u8,
}

/// What banking did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banked {
    /// Nothing to bank: the tap changes nothing.
    Nothing,
    /// The turn's points went to the player, who is now on `total`, and the
    /// die passed on.
    Passed { player: u8, points: u16, total: u16 },
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

    /// Throws the die again. The two poses, and whether the pigs landed
    /// touching, are already drawn.
    pub fn throw(&mut self, poses: [Pose; 2], touching: bool) -> Throw {
        let outcome = if touching { Outcome::Smooch } else { score(poses) };
        let mut throw = Throw {
            poses,
            outcome,
            touching,
            player: self.current,
            turn_before: self.turn,
            banked_before: self.scores[self.current as usize],
            won: false,
        };
        if self.winner.is_none() {
            match outcome {
                Outcome::Score(points) => {
                    self.turn += points;
                    // Reaching the target wins there and then: no bank.
                    let total = self.scores[self.current as usize] + self.turn;
                    if total >= TARGET {
                        self.scores[self.current as usize] = total;
                        self.turn = 0;
                        self.winner = Some(self.current);
                        throw.won = true;
                    }
                }
                Outcome::Bust => {
                    self.turn = 0;
                    self.advance();
                }
                Outcome::Smooch => {
                    self.scores[self.current as usize] = 0;
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
        self.advance();
        Banked::Passed {
            player,
            points,
            total,
        }
    }

    fn advance(&mut self) {
        self.current = (self.current + 1) % self.players;
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
        assert_eq!(throw_label([SideDot, SidePlain], false).as_str(), "Oops");
        assert_eq!(throw_label([SidePlain, SidePlain], false).as_str(), "Nap Time");
        assert_eq!(throw_label([Feet, Feet], false).as_str(), "Twin Strut");
        assert_eq!(throw_label([SideDot, Nose], false).as_str(), "Nose Dive");
        assert_eq!(throw_label([Back, Feet], false).as_str(), "Belly Up + Strut");
    }

    #[test]
    fn throws_build_the_turn_and_banking_passes_the_die() {
        let mut g = Pigs::new(3);
        g.throw([Back, Feet], false);
        g.throw([Nose, SideDot], false);
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
        g.throw([Back, Back], false);
        g.bank();
        g.throw([Feet, Feet], false);
        let t = g.throw([SideDot, SidePlain], false);
        assert_eq!(t.outcome, Outcome::Bust);
        assert_eq!(g.turn(), 0);
        assert_eq!(g.current(), 0, "back round to the first player");
        assert_eq!(g.scores(), &[20, 0]);
    }

    #[test]
    fn touching_pigs_wipe_the_whole_score() {
        let mut g = Pigs::new(2);
        g.throw([Back, Back], false);
        g.bank();
        g.throw([Back, Feet], false);
        g.bank();
        g.throw([Feet, Feet], false);
        g.bank();
        assert_eq!(g.scores(), &[40, 10]);
        assert_eq!(g.current(), 1);
        g.throw([Back, Feet], false);
        assert_eq!(g.turn(), 10);
        let t = g.throw([Nose, Nose], true);
        assert_eq!(t.outcome, Outcome::Smooch);
        assert_eq!(t.banked_before, 10, "what was wiped");
        assert_eq!(g.scores(), &[40, 0], "the whole banked score, and only theirs");
        assert_eq!(g.turn(), 0, "and the turn's points");
        assert_eq!(g.current(), 0, "and the die passes");
    }

    #[test]
    fn touching_is_rare_and_a_smooch_names_itself() {
        let n = 100_000u32;
        let touches = (0..n)
            .filter(|&i| smooch_from_random(((i as u64 * (u32::MAX as u64 + 1)) / n as u64) as u32))
            .count() as u32;
        let want = n * SMOOCH_ODDS / WEIGHT_TOTAL;
        assert!(
            touches.abs_diff(want) <= 2,
            "about one throw in a hundred: {touches}"
        );
        assert!(!smooch_from_random(u32::MAX));
        assert_eq!(throw_label([Back, Feet], true).as_str(), "Smooch");
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
            g.throw([Back, Feet], false);
            g.bank();
        }
        assert_eq!(g.current(), 0);
    }

    #[test]
    fn reaching_the_target_wins_without_a_bank_and_a_tap_starts_again() {
        let mut g = Pigs::new(2);
        for _ in 0..4 {
            assert!(!g.throw([Back, Back], false).won);
        }
        assert_eq!(g.turn(), 80);
        assert_eq!(g.winner(), None);
        let t = g.throw([Back, Back], false);
        assert!(t.won, "the throw that reaches the target wins");
        assert_eq!(g.winner(), Some(0));
        assert_eq!(g.scores(), &[100, 0], "the turn goes into the score");
        assert_eq!(g.turn(), 0);
        assert_eq!(g.current(), 0, "the die stays with the winner");
        g.throw([Back, Feet], false);
        assert_eq!(g.turn(), 0, "no throwing once it's won");
        assert_eq!(g.bank(), Banked::NewGame);
        assert_eq!(g.scores(), &[0, 0]);
        assert_eq!(g.winner(), None);
        assert_eq!(g.players(), 2);
    }

    #[test]
    fn banked_points_count_towards_the_target() {
        let mut g = Pigs::new(2);
        g.throw([Ear, Ear], false);
        g.bank();
        g.throw([Back, Feet], false);
        g.bank();
        assert_eq!(g.scores(), &[60, 10]);
        g.throw([Nose, Nose], false);
        assert_eq!(g.winner(), Some(0), "60 banked + 40 thrown");
        assert_eq!(g.scores(), &[100, 10]);
    }

    #[test]
    fn tokens_run_through_the_letters_then_the_symbols_and_wrap() {
        let a = Token::letter('A');
        assert_eq!(a.initial(), Some('A'));
        assert_eq!(a.stepped(1).initial(), Some('B'));
        let z = Token::letter('Z');
        assert_eq!(z.stepped(1).as_symbol(), Some(Symbol::Hat));
        assert_eq!(z.stepped(1).initial(), None);
        assert_eq!(a.stepped(-1).as_symbol(), Some(Symbol::Star), "down from A");
        assert_eq!(a.stepped(-1).stepped(1), a);
        assert_eq!(Token::symbol(Symbol::Car).as_symbol(), Some(Symbol::Car));
        let names = DEFAULT_TOKENS.map(|t| t.initial());
        assert_eq!(names, ['A', 'B', 'C', 'D', 'E', 'F'].map(Some));
    }

    #[test]
    fn the_player_count_is_kept_in_range() {
        assert_eq!(Pigs::new(0).players(), MIN_PLAYERS);
        assert_eq!(Pigs::new(40).players(), MAX_PLAYERS);
    }
}
