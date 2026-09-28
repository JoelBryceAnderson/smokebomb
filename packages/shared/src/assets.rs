//! Layout of the pre-rendered animation pack stored in the 64 MB QSPI flash.
//!
//! Smoke effects are rendered offline (see `docs/ARCHITECTURE.md`) and written
//! to external flash as a single pack. Theme-store downloads use the same
//! format, so the server, the phone and the die all agree on it.
//!
//! ```text
//! offset 0     PackHeader            (16 bytes)
//! offset 16    ClipEntry * clip_count (16 bytes each)
//! ...          frame data, each frame = FACE_COUNT * FRAME_BYTES, 4bpp packed
//! ```

use crate::types::FACE_COUNT;

pub const PACK_MAGIC: [u8; 4] = *b"SMKB";
pub const PACK_VERSION: u16 = 1;

/// Size of the external QSPI flash reserved for animation packs.
pub const QSPI_CAPACITY: u32 = 64 * 1024 * 1024;

pub const PANEL_WIDTH: usize = 96;
pub const PANEL_HEIGHT: usize = 96;
/// SSD1317 grayscale depth used by the pipeline.
pub const BITS_PER_PIXEL: usize = 4;
/// One panel frame: two pixels per byte, high nibble first.
pub const FRAME_BYTES: usize = PANEL_WIDTH * PANEL_HEIGHT * BITS_PER_PIXEL / 8;
/// All six faces for one animation frame.
pub const CUBE_FRAME_BYTES: usize = FRAME_BYTES * FACE_COUNT;

pub const HEADER_LEN: usize = 16;
pub const CLIP_ENTRY_LEN: usize = 16;

/// Well-known clip slots the firmware looks up by id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum ClipId {
    SmokeIdle = 0,
    SmokeShake = 1,
    SmokeThrow = 2,
    Reveal = 3,
    MaxBurst = 4,
    Dud = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackHeader {
    pub version: u16,
    pub clip_count: u16,
    /// Total pack length in bytes, header included.
    pub total_len: u32,
}

impl PackHeader {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&PACK_MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.clip_count.to_le_bytes());
        b[8..12].copy_from_slice(&self.total_len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; HEADER_LEN]) -> Option<PackHeader> {
        if b[0..4] != PACK_MAGIC {
            return None;
        }
        Some(PackHeader {
            version: u16::from_le_bytes([b[4], b[5]]),
            clip_count: u16::from_le_bytes([b[6], b[7]]),
            total_len: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipEntry {
    pub id: u16,
    pub frame_count: u16,
    pub fps: u8,
    pub flags: u8,
    /// Absolute byte offset of the first frame within the pack.
    pub offset: u32,
}

impl ClipEntry {
    pub const FLAG_LOOP: u8 = 1 << 0;

    pub fn encode(&self) -> [u8; CLIP_ENTRY_LEN] {
        let mut b = [0u8; CLIP_ENTRY_LEN];
        b[0..2].copy_from_slice(&self.id.to_le_bytes());
        b[2..4].copy_from_slice(&self.frame_count.to_le_bytes());
        b[4] = self.fps;
        b[5] = self.flags;
        b[8..12].copy_from_slice(&self.offset.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; CLIP_ENTRY_LEN]) -> ClipEntry {
        ClipEntry {
            id: u16::from_le_bytes([b[0], b[1]]),
            frame_count: u16::from_le_bytes([b[2], b[3]]),
            fps: b[4],
            flags: b[5],
            offset: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
        }
    }

    pub fn frame_offset(&self, frame: u16) -> u32 {
        self.offset + frame as u32 * CUBE_FRAME_BYTES as u32
    }

    pub fn looping(&self) -> bool {
        self.flags & Self::FLAG_LOOP != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget() {
        // 64 MB holds ~2400 six-face frames: ~80 s of smoke at 30 fps.
        assert_eq!(FRAME_BYTES, 4608);
        assert!(QSPI_CAPACITY as usize / CUBE_FRAME_BYTES > 2400);
    }

    #[test]
    fn roundtrip() {
        let h = PackHeader {
            version: PACK_VERSION,
            clip_count: 3,
            total_len: 99,
        };
        assert_eq!(PackHeader::decode(&h.encode()), Some(h));
        let c = ClipEntry {
            id: 2,
            frame_count: 40,
            fps: 30,
            flags: ClipEntry::FLAG_LOOP,
            offset: 64,
        };
        assert_eq!(ClipEntry::decode(&c.encode()), c);
    }
}
