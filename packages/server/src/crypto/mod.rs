//! Roll signature verification.
//!
//! Dice sign `SHA-256(RollRecord::encode())` with their ATECC608 P-256 key.
//! The server recomputes the digest from the submitted fields, so a client
//! cannot pair a valid signature with altered values.

use p256::ecdsa::signature::hazmat::PrehashVerifier;
use p256::ecdsa::{Signature, VerifyingKey};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error("stored public key is malformed")]
    BadKey,
    #[error("signature is malformed")]
    BadSignature,
    #[error("signature does not match")]
    Mismatch,
}

/// `public_key` is raw X || Y (64 bytes) as reported by the ATECC608.
pub fn verify_digest(public_key: &[u8], digest: &[u8; 32], signature: &[u8; 64]) -> Result<(), VerifyError> {
    if public_key.len() != 64 {
        return Err(VerifyError::BadKey);
    }
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(public_key);
    let key = VerifyingKey::from_sec1_bytes(&sec1).map_err(|_| VerifyError::BadKey)?;
    let sig = Signature::from_slice(signature).map_err(|_| VerifyError::BadSignature)?;
    key.verify_prehash(digest, &sig)
        .map_err(|_| VerifyError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::signature::hazmat::PrehashSigner;
    use p256::ecdsa::SigningKey;

    #[test]
    fn roundtrip() {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let pk = &sk.verifying_key().to_encoded_point(false).as_bytes()[1..].to_vec();
        let digest = [42u8; 32];
        let sig: Signature = sk.sign_prehash(&digest).unwrap();
        let sig: [u8; 64] = sig.to_bytes().into();

        assert_eq!(verify_digest(pk, &digest, &sig), Ok(()));
        assert_eq!(verify_digest(pk, &[0u8; 32], &sig), Err(VerifyError::Mismatch));
        assert_eq!(verify_digest(&pk[..10], &digest, &sig), Err(VerifyError::BadKey));
    }
}
