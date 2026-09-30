//! BLE GATT message definitions between the die and the phone app.
//!
//! Framing (length prefix, fragmentation over the ATT MTU) lives in the
//! firmware comms layer; these are the payloads. The Kotlin mirror is
//! `packages/mobile/shared/.../SharedModels.kt`.

use serde::{Deserialize, Serialize};

use crate::modes::{License, ModeId, ModeSet};
use crate::roll::SignedRoll;
use crate::types::{DieKind, SessionId};

/// GATT service UUID for the Sugarcube service (placeholder, not yet allocated).
pub const SERVICE_UUID: &str = "5b0e0000-5b0e-4d1e-9a5e-736d6f6b6562";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DieToPhone {
    Hello {
        firmware_version: (u8, u8, u8),
        battery_percent: u8,
    },
    Roll(SignedRoll),
    /// Response to [`PhoneToDie::SyncHistory`]; `None` marks the end.
    HistoryItem(Option<SignedRoll>),
    PublicKey(#[serde(with = "pubkey_serde")] [u8; 64]),
    /// Response to [`PhoneToDie::GetInventory`], and sent again whenever it
    /// changes (a license installed, modes changed from the phone, a mode
    /// picked on the die).
    Inventory(Inventory),
}

/// What a die can play and what it's set up to play.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// Modes the die holds a license for (Dice always).
    pub licensed: ModeSet,
    /// Licensed modes the owner has turned on: the Mode page offers these.
    pub enabled: ModeSet,
    /// The mode a throw plays now.
    pub active: ModeId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PhoneToDie {
    SetOwnerName(heapless::String<24>),
    SetDie {
        kind: DieKind,
        count: u8,
    },
    JoinSession(SessionId),
    LeaveSession,
    SyncHistory {
        since_counter: u32,
    },
    GetPublicKey,
    GetInventory,
    /// Which licensed modes the Mode page offers. Unlicensed modes are
    /// ignored and Dice is always kept; one mode left hides the Mode page.
    SetEnabledModes(ModeSet),
    /// Unlock a store item on this die (see docs/STORE.md).
    InstallLicense(License),
}

mod pubkey_serde {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(k: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(k)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        let bytes: heapless::Vec<u8, 64> = serde::Deserialize::deserialize(d)?;
        bytes
            .into_array()
            .map_err(|_| serde::de::Error::custom("public key must be 64 bytes"))
    }
}
