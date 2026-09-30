//! Types shared by every Rust component of Sugarcube.
//!
//! This crate is `no_std` and allocation-free so the firmware can use it
//! directly; the server and simulator use the same definitions, which keeps
//! the signed roll format byte-for-byte identical on both ends.

#![no_std]

pub mod assets;
pub mod protocol;
pub mod roll;
pub mod types;

pub use roll::{RollRecord, SignedRoll};
pub use types::{DeviceSerial, DieKind, Face, PotFace, SessionId};
