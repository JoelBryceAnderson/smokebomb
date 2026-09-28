//! Signed roll record.
//!
//! Every roll produces a [`RollRecord`]. Its canonical encoding is hashed with
//! SHA-256 and the digest is signed by the ATECC608 (P-256 ECDSA). Each record
//! carries the digest of the previous one, so a device's roll history forms a
//! hash chain the server can audit for gaps or rewrites.

use heapless::Vec;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{DeviceSerial, DieKind, SessionId, MAX_DICE};

/// Format version, bumped whenever [`RollRecord::encode`] or the meaning of
/// its fields changes. v2: up to 10 dice and the Pass the Pot die kind.
pub const ROLL_FORMAT_VERSION: u8 = 2;

/// Upper bound on [`RollRecord::encode`] output.
pub const ROLL_ENCODED_MAX: usize = 1 + 9 + 16 + 4 + 8 + 1 + 1 + MAX_DICE + 32;

/// Digest used as `prev_hash` for a device's first roll.
pub const GENESIS_HASH: [u8; 32] = [0; 32];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollRecord {
    pub device: DeviceSerial,
    pub session: SessionId,
    /// Monotonic per-device counter (ATECC608 monotonic counter on hardware).
    pub counter: u32,
    /// Milliseconds since device boot. Wall-clock time is attached by the
    /// phone/server, not trusted from the die.
    pub uptime_ms: u64,
    pub die: DieKind,
    /// Raw values, each in `1..=die.sides()`. For Pass the Pot these are d6
    /// values; the face shown is [`crate::PotFace::from_raw`].
    pub values: Vec<u8, MAX_DICE>,
    pub prev_hash: [u8; 32],
}

impl RollRecord {
    /// Canonical, fixed-order little-endian encoding. This is the exact byte
    /// string that gets hashed and signed; do not reorder fields.
    pub fn encode(&self) -> Vec<u8, ROLL_ENCODED_MAX> {
        let mut out: Vec<u8, ROLL_ENCODED_MAX> = Vec::new();
        // Capacity is sized for the worst case above, so these cannot fail.
        let _ = out.push(ROLL_FORMAT_VERSION);
        let _ = out.extend_from_slice(&self.device.0);
        let _ = out.extend_from_slice(&self.session.0);
        let _ = out.extend_from_slice(&self.counter.to_le_bytes());
        let _ = out.extend_from_slice(&self.uptime_ms.to_le_bytes());
        let _ = out.push(self.die as u8);
        let _ = out.push(self.values.len() as u8);
        let _ = out.extend_from_slice(&self.values);
        let _ = out.extend_from_slice(&self.prev_hash);
        out
    }

    /// SHA-256 of [`encode`](Self::encode); the value the secure element signs.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    pub fn total(&self) -> u16 {
        self.values.iter().map(|&v| v as u16).sum()
    }
}

/// A roll record plus its raw P-256 signature (r || s, 64 bytes).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedRoll {
    pub record: RollRecord,
    #[serde(with = "sig_serde")]
    pub signature: [u8; 64],
}

impl SignedRoll {
    /// Check that `self` directly follows `prev` in the device's hash chain.
    pub fn follows(&self, prev: &RollRecord) -> bool {
        self.record.device == prev.device
            && self.record.counter == prev.counter.wrapping_add(1)
            && self.record.prev_hash == prev.digest()
    }
}

/// serde only derives arrays up to 32 elements; encode the signature as a
/// byte sequence instead.
mod sig_serde {
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(sig: &[u8; 64], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(sig)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 64], D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = [u8; 64];
            fn expecting(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                f.write_str("64 signature bytes")
            }
            fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<[u8; 64], E> {
                v.try_into().map_err(|_| E::invalid_length(v.len(), &self))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; 64], A::Error> {
                let mut out = [0u8; 64];
                for (i, b) in out.iter_mut().enumerate() {
                    *b = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(i, &self))?;
                }
                Ok(out)
            }
        }
        d.deserialize_bytes(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(counter: u32, prev_hash: [u8; 32]) -> RollRecord {
        RollRecord {
            device: DeviceSerial([1; 9]),
            session: SessionId::NONE,
            counter,
            uptime_ms: 1234,
            die: DieKind::D20,
            values: Vec::from_slice(&[17, 3]).unwrap(),
            prev_hash,
        }
    }

    #[test]
    fn encoding_is_stable() {
        let r = record(7, GENESIS_HASH);
        let enc = r.encode();
        assert_eq!(enc.len(), 1 + 9 + 16 + 4 + 8 + 1 + 1 + 2 + 32);
        assert_eq!(enc[0], ROLL_FORMAT_VERSION);
        assert_eq!(r.total(), 20);
    }

    #[test]
    fn ten_dice_fit() {
        let mut r = record(0, GENESIS_HASH);
        r.values = Vec::from_slice(&[1; MAX_DICE]).unwrap();
        assert_eq!(r.encode().len(), ROLL_ENCODED_MAX);
    }

    #[test]
    fn hash_chain_links() {
        let first = record(0, GENESIS_HASH);
        let second = SignedRoll {
            record: record(1, first.digest()),
            signature: [0; 64],
        };
        assert!(second.follows(&first));

        let forged = SignedRoll {
            record: record(1, GENESIS_HASH),
            signature: [0; 64],
        };
        assert!(!forged.follows(&first));
    }
}
