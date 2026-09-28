use serde::{Deserialize, Serialize};

/// Number of OLED faces on the die.
pub const FACE_COUNT: usize = 6;

/// Most dice a single Smokebomb can roll at once (set from the on-die menu).
pub const MAX_DICE: usize = 10;

/// Pass the Pot uses at most three dice, like the table game.
pub const MAX_POT_DICE: usize = 3;

/// Physical face of the cube, named by the body-frame axis its outward
/// normal points along. The discriminant order matches three.js
/// `BoxGeometry` material order, so the simulator UI can index faces directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Face {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
    pub const ALL: [Face; FACE_COUNT] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn from_index(i: usize) -> Option<Face> {
        if i < FACE_COUNT {
            Some(Self::ALL[i])
        } else {
            None
        }
    }

    pub const fn opposite(self) -> Face {
        match self {
            Face::PosX => Face::NegX,
            Face::NegX => Face::PosX,
            Face::PosY => Face::NegY,
            Face::NegY => Face::PosY,
            Face::PosZ => Face::NegZ,
            Face::NegZ => Face::PosZ,
        }
    }
}

/// Which die the Smokebomb is emulating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DieKind {
    D4 = 0,
    D6 = 1,
    D8 = 2,
    D10 = 3,
    D12 = 4,
    D20 = 5,
    D100 = 6,
    /// Pass the Pot: each die shows ←, P, → or • (see [`PotFace`]). It is
    /// rolled and signed as a plain d6; the face is derived from the value.
    PassThePot = 7,
}

impl DieKind {
    /// Menu order (SIM_SPEC C3, "Which die").
    pub const ALL: [DieKind; 8] = [
        DieKind::D4,
        DieKind::D6,
        DieKind::D8,
        DieKind::D10,
        DieKind::D12,
        DieKind::D20,
        DieKind::D100,
        DieKind::PassThePot,
    ];

    /// Range of the raw value drawn for each die: `1..=sides()`.
    pub const fn sides(self) -> u8 {
        match self {
            DieKind::D4 => 4,
            DieKind::D6 | DieKind::PassThePot => 6,
            DieKind::D8 => 8,
            DieKind::D10 => 10,
            DieKind::D12 => 12,
            DieKind::D20 => 20,
            DieKind::D100 => 100,
        }
    }

    pub const fn is_numeric(self) -> bool {
        !matches!(self, DieKind::PassThePot)
    }

    /// Most dice allowed with this die kind.
    pub const fn max_count(self) -> usize {
        if self.is_numeric() {
            MAX_DICE
        } else {
            MAX_POT_DICE
        }
    }

    /// Stable name used in the REST API and database.
    pub const fn wire_name(self) -> &'static str {
        match self {
            DieKind::D4 => "d4",
            DieKind::D6 => "d6",
            DieKind::D8 => "d8",
            DieKind::D10 => "d10",
            DieKind::D12 => "d12",
            DieKind::D20 => "d20",
            DieKind::D100 => "d100",
            DieKind::PassThePot => "pass_the_pot",
        }
    }

    pub fn from_wire(name: &str) -> Option<DieKind> {
        Self::ALL.into_iter().find(|d| d.wire_name() == name)
    }

    pub const fn from_u8(v: u8) -> Option<DieKind> {
        if (v as usize) < Self::ALL.len() {
            Some(Self::ALL[v as usize])
        } else {
            None
        }
    }
}

/// A Pass the Pot face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PotFace {
    /// ← pass a chip left (1 in 6)
    Left,
    /// P put a chip in the pot (1 in 6)
    Pot,
    /// → pass a chip right (1 in 6)
    Right,
    /// • keep (1 in 2)
    Keep,
}

impl PotFace {
    /// Map a raw d6 value (1..=6) to its face, exactly as the mockup does:
    /// 1 → ←, 2 → P, 3 → →, 4–6 → •.
    pub const fn from_raw(value: u8) -> PotFace {
        match value {
            1 => PotFace::Left,
            2 => PotFace::Pot,
            3 => PotFace::Right,
            _ => PotFace::Keep,
        }
    }

    pub const fn glyph(self) -> char {
        match self {
            PotFace::Left => '←',
            PotFace::Pot => 'P',
            PotFace::Right => '→',
            PotFace::Keep => '•',
        }
    }
}

/// ATECC608 serial number (9 bytes, burned in at the factory).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeviceSerial(pub [u8; 9]);

/// Verified-session identifier handed to the die over NFC/BLE when it joins
/// an organizer's table. All zeroes means "casual roll, no session".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub [u8; 16]);

impl SessionId {
    pub const NONE: SessionId = SessionId([0; 16]);

    pub fn is_none(&self) -> bool {
        self.0 == [0; 16]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        for d in DieKind::ALL {
            assert_eq!(DieKind::from_wire(d.wire_name()), Some(d));
            assert_eq!(DieKind::from_u8(d as u8), Some(d));
        }
        assert_eq!(DieKind::from_wire("d7"), None);
    }

    #[test]
    fn pot_faces_follow_the_mockup_odds() {
        let faces: [PotFace; 6] = core::array::from_fn(|i| PotFace::from_raw(i as u8 + 1));
        assert_eq!(&faces[..3], &[PotFace::Left, PotFace::Pot, PotFace::Right]);
        assert!(faces[3..].iter().all(|f| *f == PotFace::Keep));
        assert_eq!(DieKind::PassThePot.max_count(), MAX_POT_DICE);
    }
}
