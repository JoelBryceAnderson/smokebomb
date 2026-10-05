//! What table screens say, on any panel (brief 3, 1.2).
//!
//! A table screen is read from across the table: T1, the answer in 1–3
//! characters or one glyph, in a colour that means the same in every game,
//! and at most one T2 word of detail. These are the words; each panel draws
//! them at its own tier sizes.

use core::fmt::Write;

use heapless::String;
use smokebomb_shared::{DieKind, PotFace};

use crate::pigs::{Outcome, Pose, Throw};

/// A roll's total as T1: `1k` for 1000, the one total too long for three
/// characters (ten d100s, every one 100: always a max).
pub fn total(n: u16) -> String<4> {
    let mut s = String::new();
    let _ = if n >= 1000 {
        write!(s, "{}k", n / 1000)
    } else {
        write!(s, "{n}")
    };
    s
}

/// The dice setup as one T2 word: `d20`, `3d6`, `10d20`, and for several
/// d100s `10d%`, so it never passes five characters.
pub fn dice(die: DieKind, count: u8) -> String<6> {
    let mut s = String::new();
    let name = die.wire_name();
    let _ = match count {
        1 => write!(s, "{name}"),
        n if die == DieKind::D100 => write!(s, "{n}d%"),
        n => write!(s, "{n}{name}"),
    };
    s
}

/// A pose's name in five characters at most.
pub const fn pose(p: Pose) -> &'static str {
    match p {
        Pose::SideDot | Pose::SidePlain => "Nap",
        Pose::Back => "Belly",
        Pose::Feet => "Strut",
        Pose::Nose => "Dive",
        Pose::Ear => "Tipsy",
    }
}

/// What a scoring throw was, as one T2 word: `Twin`, `Nap`, or the pose
/// that scored most.
pub fn throw_word(t: &Throw) -> &'static str {
    let [a, b] = t.poses;
    match t.outcome {
        Outcome::Bust => "OOPS",
        Outcome::Smooch => "kiss",
        Outcome::Score(_) if a.is_side() && b.is_side() => "Nap",
        Outcome::Score(_) if a == b => "Twin",
        Outcome::Score(_) if a.is_side() => pose(b),
        Outcome::Score(_) if b.is_side() => pose(a),
        Outcome::Score(_) => {
            if a.points() >= b.points() {
                pose(a)
            } else {
                pose(b)
            }
        }
    }
}

/// A Pass the Pot result's T2: `keep` when nothing moves, else how many
/// bills leave the player's hand, `−2`.
pub fn pot(values: &[u8]) -> String<6> {
    let gone = values
        .iter()
        .filter(|&&v| PotFace::from_raw(v) != PotFace::Keep)
        .count();
    let mut s = String::new();
    let _ = if gone == 0 {
        s.push_str("keep").map_err(|_| core::fmt::Error)
    } else {
        write!(s, "−{gone}")
    };
    s
}

/// A signed number for T1: `+15`, `−40`, `0`.
pub fn signed(n: i32) -> String<5> {
    let mut s = String::new();
    let _ = match n {
        0 => write!(s, "0"),
        n if n > 0 => write!(s, "+{n}"),
        n => write!(s, "−{}", n.unsigned_abs()),
    };
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pigs::Pose::*;

    #[test]
    fn words_fit_their_tiers() {
        assert_eq!(total(1000).as_str(), "1k");
        assert_eq!(total(999).as_str(), "999");
        assert_eq!(dice(DieKind::D20, 1).as_str(), "d20");
        assert_eq!(dice(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(dice(DieKind::D100, 1).as_str(), "d100");
        assert_eq!(dice(DieKind::D100, 10).as_str(), "10d%");
        for die in DieKind::NUMERIC {
            for n in 1..=smokebomb_shared::types::MAX_DICE as u8 {
                assert!(dice(die, n).chars().count() <= 5, "{n}{die:?}");
            }
        }
        for p in Pose::ALL {
            assert!(pose(p).chars().count() <= 5);
        }
        assert_eq!(pot(&[4, 5]).as_str(), "keep");
        assert_eq!(pot(&[1, 2, 6]).as_str(), "−2");
        assert_eq!(signed(15).as_str(), "+15");
        assert_eq!(signed(-40).as_str(), "−40");
    }

    #[test]
    fn a_throw_is_named_by_what_scored() {
        let throw = |poses: [Pose; 2]| Throw {
            poses,
            outcome: crate::pigs::score(poses),
            touching: false,
            player: 0,
            turn_before: 0,
            banked_before: 0,
            won: false,
        };
        assert_eq!(throw_word(&throw([Back, Back])), "Twin");
        assert_eq!(throw_word(&throw([SideDot, SideDot])), "Nap");
        assert_eq!(throw_word(&throw([SideDot, Feet])), "Strut");
        assert_eq!(throw_word(&throw([Back, Ear])), "Tipsy");
        assert_eq!(throw_word(&throw([SideDot, SidePlain])), "OOPS");
    }
}
