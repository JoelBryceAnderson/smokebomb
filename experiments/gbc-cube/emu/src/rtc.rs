//! The MBC3 real-time clock in the `.sav` file.
//!
//! Crystal keeps its day/night clock in the cartridge's RTC, so a save
//! without it would come back to the wrong time of day. The save file is the
//! cart RAM followed by this crate's own 24-byte footer:
//!
//! | Bytes | |
//! |---|---|
//! | 0–4 | RTC registers: seconds, minutes, hours, day low, day high/flags |
//! | 5–7 | zero |
//! | 8–15 | Unix time of the write, little endian |
//! | 16–23 | `GCBRTC1\0` |
//!
//! On load the clock is advanced by the wall time that passed, so the game
//! sees the time move while the cube was off (as the cartridge's battery
//! would). Other emulators ignore the trailing bytes.

const MAGIC: &[u8; 8] = b"GCBRTC1\0";
pub const FOOTER_LEN: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Footer {
    pub regs: [u8; 5],
    pub unix_secs: u64,
}

impl Footer {
    pub fn parse(bytes: &[u8]) -> Option<Footer> {
        if bytes.len() < FOOTER_LEN || &bytes[16..24] != MAGIC {
            return None;
        }
        let mut regs = [0; 5];
        regs.copy_from_slice(&bytes[..5]);
        let unix_secs = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
        Some(Footer { regs, unix_secs })
    }

    pub fn to_bytes(&self) -> [u8; FOOTER_LEN] {
        let mut b = [0u8; FOOTER_LEN];
        b[..5].copy_from_slice(&self.regs);
        b[8..16].copy_from_slice(&self.unix_secs.to_le_bytes());
        b[16..24].copy_from_slice(MAGIC);
        b
    }

    /// The registers advanced to `now` (no change if the clock is halted or
    /// `now` is earlier than the write).
    pub fn advanced_to(&self, now: u64) -> [u8; 5] {
        let r = self.regs;
        if r[4] & 0x40 != 0 || now <= self.unix_secs {
            return r;
        }
        let day = (((r[4] & 1) as u64) << 8) | r[3] as u64;
        let total = day * 86_400 + r[2] as u64 * 3600 + r[1] as u64 * 60 + r[0] as u64;
        let t = total + (now - self.unix_secs);
        let day = t / 86_400;
        let mut high = r[4] & 0xC0;
        if day > 511 {
            high |= 0x80; // day counter carry
        }
        let day = day % 512;
        high |= (day >> 8) as u8;
        [
            (t % 60) as u8,
            ((t / 60) % 60) as u8,
            ((t / 3600) % 24) as u8,
            day as u8,
            high,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let f = Footer {
            regs: [1, 2, 3, 4, 0x41],
            unix_secs: 1_700_000_000,
        };
        assert_eq!(Footer::parse(&f.to_bytes()), Some(f));
        assert_eq!(Footer::parse(&[0; 24]), None);
    }

    #[test]
    fn advances_and_carries_into_days() {
        // 23:59:30 on day 255, then 45 s later: 00:00:15 on day 256.
        let f = Footer {
            regs: [30, 59, 23, 255, 0],
            unix_secs: 1000,
        };
        assert_eq!(f.advanced_to(1045), [15, 0, 0, 0, 1]);
        // Halted clocks stay put.
        let f = Footer {
            regs: [30, 59, 23, 255, 0x40],
            unix_secs: 1000,
        };
        assert_eq!(f.advanced_to(5000), f.regs);
        // Day 511 rolls over to 0 with the carry flag.
        let f = Footer {
            regs: [0, 0, 0, 255, 1],
            unix_secs: 0,
        };
        assert_eq!(f.advanced_to(86_400), [0, 0, 0, 0, 0x80]);
    }
}
