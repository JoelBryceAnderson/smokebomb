//! Roll generation and signing.

use heapless::Vec;
use smokebomb_hal::{HalResult, Rng, SecureElement};
use smokebomb_shared::roll::GENESIS_HASH;
use smokebomb_shared::types::MAX_DICE;
use smokebomb_shared::{DeviceSerial, DieKind, RollRecord, SessionId, SignedRoll};

pub struct RollEngine {
    device: DeviceSerial,
    session: SessionId,
    /// Digest of the previous roll. Must survive reboots on hardware (TODO:
    /// persist alongside the ATECC608 counter) or the chain will break.
    prev_hash: [u8; 32],
}

impl RollEngine {
    pub fn new<S: SecureElement>(se: &mut S) -> HalResult<Self> {
        Ok(Self {
            device: DeviceSerial(se.serial()?),
            session: SessionId::NONE,
            prev_hash: GENESIS_HASH,
        })
    }

    pub fn set_session(&mut self, session: SessionId) {
        self.session = session;
    }

    pub fn roll<R: Rng, S: SecureElement>(
        &mut self,
        rng: &mut R,
        se: &mut S,
        die: DieKind,
        count: u8,
        uptime_ms: u64,
    ) -> HalResult<SignedRoll> {
        let count = (count as usize).clamp(1, die.max_count());
        let mut values: Vec<u8, MAX_DICE> = Vec::new();
        for _ in 0..count {
            let _ = values.push(uniform(rng, die.sides())?);
        }

        let record = RollRecord {
            device: self.device,
            session: self.session,
            counter: se.next_counter()?,
            uptime_ms,
            die,
            values,
            prev_hash: self.prev_hash,
        };
        let digest = record.digest();
        let signature = se.sign_digest(&digest)?;
        self.prev_hash = digest;
        Ok(SignedRoll { record, signature })
    }
}

/// Unbiased value in `1..=sides` via rejection sampling (plain `% sides` would
/// favour low faces).
pub fn uniform<R: Rng>(rng: &mut R, sides: u8) -> HalResult<u8> {
    let sides = sides as u32;
    let zone = u32::MAX - (u32::MAX % sides);
    loop {
        let x = rng.next_u32()?;
        if x < zone {
            return Ok((x % sides) as u8 + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::hazmat::PrehashVerifier;
    use p256::ecdsa::{Signature, VerifyingKey};
    use smokebomb_hal_simulator::{SimRng, SimSecureElement};

    #[test]
    fn values_in_range() {
        let mut rng = SimRng::default();
        for die in DieKind::ALL {
            for _ in 0..200 {
                let v = uniform(&mut rng, die.sides()).unwrap();
                assert!((1..=die.sides()).contains(&v));
            }
        }
    }

    #[test]
    fn rolls_are_signed_and_chained() {
        let mut rng = SimRng::default();
        let mut se = SimSecureElement::new();
        let mut engine = RollEngine::new(&mut se).unwrap();

        let first = engine.roll(&mut rng, &mut se, DieKind::D20, 2, 10).unwrap();
        let second = engine.roll(&mut rng, &mut se, DieKind::D20, 2, 20).unwrap();
        assert!(second.follows(&first.record));

        let mut sec1 = [0u8; 65];
        sec1[0] = 0x04;
        sec1[1..].copy_from_slice(&se.public_key().unwrap());
        let key = VerifyingKey::from_sec1_bytes(&sec1).unwrap();
        let sig = Signature::from_slice(&second.signature).unwrap();
        key.verify_prehash(&second.record.digest(), &sig).unwrap();
    }
}
