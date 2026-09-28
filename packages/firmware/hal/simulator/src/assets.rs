//! Contents of the simulated QSPI flash.

/// The standard asset pack (fonts + placeholder clips), built at compile time
/// by `smokebomb-assets-build`.
pub static STANDARD_PACK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pack.smkb"));

/// A fresh copy of the standard pack.
pub fn standard_pack() -> Vec<u8> {
    STANDARD_PACK.to_vec()
}
