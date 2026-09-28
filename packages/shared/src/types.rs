use serde::{Deserialize, Serialize};

/// Number of OLED faces on the die.
pub const FACE_COUNT: usize = 6;

/// Most dice a single Smokebomb can roll at once (set from the on-die menu).
pub const MAX_DICE: usize = 6;

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

/// Which polyhedral die the Smokebomb is emulating.
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
}

impl DieKind {
    pub const ALL: [DieKind; 7] = [
        DieKind::D4,
        DieKind::D6,
        DieKind::D8,
        DieKind::D10,
        DieKind::D12,
        DieKind::D20,
        DieKind::D100,
    ];

    pub const fn sides(self) -> u8 {
        match self {
            DieKind::D4 => 4,
            DieKind::D6 => 6,
            DieKind::D8 => 8,
            DieKind::D10 => 10,
            DieKind::D12 => 12,
            DieKind::D20 => 20,
            DieKind::D100 => 100,
        }
    }

    pub fn from_sides(sides: u8) -> Option<DieKind> {
        Self::ALL.into_iter().find(|d| d.sides() == sides)
    }

    pub const fn from_u8(v: u8) -> Option<DieKind> {
        if (v as usize) < Self::ALL.len() {
            Some(Self::ALL[v as usize])
        } else {
            None
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
