//! Game modes as the store, the phone and the die name them.
//!
//! Every mode's code ships in the firmware; a die plays the ones it holds a
//! license for, and the owner picks which of those the on-die Mode page
//! offers (see docs/STORE.md). Dice is always licensed and always on.

use serde::{Deserialize, Serialize};

use crate::types::DeviceSerial;

/// A game mode. The discriminant is the mode's bit in a [`ModeSet`] and
/// never changes; new modes take the next number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ModeId {
    Dice = 0,
    PassThePot = 1,
    HotPotato = 2,
    PigToss = 3,
    SugarRun = 4,
}

impl ModeId {
    /// Every mode, in wire order (also the Mode page's order).
    pub const ALL: [ModeId; 5] = [
        ModeId::Dice,
        ModeId::PassThePot,
        ModeId::HotPotato,
        ModeId::PigToss,
        ModeId::SugarRun,
    ];

    /// Stable name used in the REST API, the store catalog and licenses.
    pub const fn wire_name(self) -> &'static str {
        match self {
            ModeId::Dice => "dice",
            ModeId::PassThePot => "pass_the_pot",
            ModeId::HotPotato => "hot_potato",
            ModeId::PigToss => "pig_toss",
            ModeId::SugarRun => "sugar_run",
        }
    }

    pub fn from_wire(name: &str) -> Option<ModeId> {
        Self::ALL.into_iter().find(|m| m.wire_name() == name)
    }

    pub const fn from_u8(v: u8) -> Option<ModeId> {
        if (v as usize) < Self::ALL.len() {
            Some(Self::ALL[v as usize])
        } else {
            None
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

/// A set of modes, one bit per [`ModeId`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModeSet(pub u16);

impl ModeSet {
    pub const EMPTY: ModeSet = ModeSet(0);
    /// Dice alone: the plain dice menu, with no Mode page.
    pub const DICE: ModeSet = ModeSet(ModeId::Dice.bit());
    /// Every mode this build knows.
    pub const ALL: ModeSet = ModeSet((1 << ModeId::ALL.len()) - 1);

    pub const fn contains(self, m: ModeId) -> bool {
        self.0 & m.bit() != 0
    }

    pub const fn with(self, m: ModeId) -> ModeSet {
        ModeSet(self.0 | m.bit())
    }

    pub const fn without(self, m: ModeId) -> ModeSet {
        ModeSet(self.0 & !m.bit())
    }

    pub const fn intersect(self, other: ModeSet) -> ModeSet {
        ModeSet(self.0 & other.0)
    }

    pub const fn len(self) -> usize {
        (self.0 & Self::ALL.0).count_ones() as usize
    }

    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }

    /// The known modes in the set, in wire order. Unknown bits (modes from a
    /// newer firmware) are skipped.
    pub fn iter(self) -> impl Iterator<Item = ModeId> {
        ModeId::ALL.into_iter().filter(move |m| self.contains(*m))
    }
}

impl FromIterator<ModeId> for ModeSet {
    fn from_iter<I: IntoIterator<Item = ModeId>>(iter: I) -> Self {
        iter.into_iter().fold(ModeSet::EMPTY, ModeSet::with)
    }
}

/// Permission for one die to play one store item, signed by the store.
///
/// The server issues it after a purchase; the phone hands it to the die,
/// which checks the signature against the store key built into the firmware
/// and that `device` is its own serial. The die doesn't check it yet (see
/// docs/STORE.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct License {
    pub device: DeviceSerial,
    pub item: LicensedItem,
    /// Seconds since the Unix epoch. Informational: the die has no trusted
    /// clock, and a license doesn't expire.
    pub issued_at: u64,
    /// Raw P-256 `r || s` over [`License::message`].
    #[serde(with = "sig_serde")]
    pub signature: [u8; 64],
}

/// What a [`License`] unlocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LicensedItem {
    Mode(ModeId),
    /// A theme, by its asset pack's SHA-256.
    Theme([u8; 32]),
}

impl License {
    /// Domain tag at the front of every signed license message.
    pub const TAG: &'static [u8; 8] = b"SCLIC\x00\x00\x01";

    /// The bytes the store signs: tag, serial, item kind and id, issue time.
    pub fn message(&self) -> heapless::Vec<u8, 64> {
        let mut m = heapless::Vec::new();
        let _ = m.extend_from_slice(Self::TAG);
        let _ = m.extend_from_slice(&self.device.0);
        match self.item {
            LicensedItem::Mode(id) => {
                let _ = m.push(0);
                let _ = m.push(id as u8);
            }
            LicensedItem::Theme(sha) => {
                let _ = m.push(1);
                let _ = m.extend_from_slice(&sha);
            }
        }
        let _ = m.extend_from_slice(&self.issued_at.to_be_bytes());
        m
    }
}

mod sig_serde {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(k: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(k)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let bytes: heapless::Vec<u8, 64> = serde::Deserialize::deserialize(d)?;
        bytes
            .into_array()
            .map_err(|_| serde::de::Error::custom("signature must be 64 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        for m in ModeId::ALL {
            assert_eq!(ModeId::from_wire(m.wire_name()), Some(m));
            assert_eq!(ModeId::from_u8(m as u8), Some(m));
        }
        assert_eq!(ModeId::from_wire("chess"), None);
        assert_eq!(ModeId::from_u8(ModeId::ALL.len() as u8), None);
    }

    #[test]
    fn sets() {
        assert_eq!(ModeSet::ALL.len(), ModeId::ALL.len());
        assert!(ModeSet::DICE.contains(ModeId::Dice));
        assert_eq!(ModeSet::DICE.len(), 1);
        let s = ModeSet::DICE.with(ModeId::PigToss);
        assert!(s.contains(ModeId::PigToss) && !s.contains(ModeId::HotPotato));
        assert_eq!(s.without(ModeId::PigToss), ModeSet::DICE);
        assert_eq!(
            s.iter().collect::<heapless::Vec<_, 4>>(),
            [ModeId::Dice, ModeId::PigToss]
        );
        assert_eq!(s.iter().collect::<ModeSet>(), s);
        // A newer firmware's mode is ignored, not counted.
        assert_eq!(ModeSet(1 << 15).len(), 0);
        assert_eq!(ModeSet::ALL.intersect(ModeSet::DICE), ModeSet::DICE);
    }

    #[test]
    fn license_messages_differ_by_serial_and_item() {
        let l = License {
            device: DeviceSerial([1; 9]),
            item: LicensedItem::Mode(ModeId::PigToss),
            issued_at: 1_790_000_000,
            signature: [0; 64],
        };
        let m = l.message();
        assert_eq!(&m[..8], License::TAG);
        assert_eq!(m.len(), 8 + 9 + 2 + 8);
        let other_die = License {
            device: DeviceSerial([2; 9]),
            ..l.clone()
        };
        assert_ne!(other_die.message(), m);
        let other_mode = License {
            item: LicensedItem::Mode(ModeId::HotPotato),
            ..l.clone()
        };
        assert_ne!(other_mode.message(), m);
        let theme = License {
            item: LicensedItem::Theme([7; 32]),
            ..l
        };
        assert_eq!(theme.message().len(), 8 + 9 + 1 + 32 + 8);
    }
}
