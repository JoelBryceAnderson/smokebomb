//! Layout of the asset pack stored in the 64 MB QSPI flash.
//!
//! The pack holds everything the firmware draws from flash: Space Grotesk
//! bitmaps at every size the screens use, smoke sprites (with the particle
//! system), and the legacy placeholder smoke clips. Theme-store downloads use
//! the same format, so the server, the phone and the die all agree on it.
//!
//! ```text
//! offset 0     PackHeader                   (16 bytes)
//! offset 16    SectionEntry * section_count (16 bytes each)
//! ...          sections, at the offsets their entries give
//! ```
//!
//! Section kinds:
//! - [`SectionKind::Clips`]: `u16 clip_count, u16 reserved`, then
//!   [`ClipEntry`] × clip_count. Clip frame offsets are absolute within the
//!   pack; each frame is `FACE_COUNT * FRAME_BYTES`, 4bpp packed.
//! - [`SectionKind::Font`]: one weight at one size. [`FontHeader`], then
//!   [`GlyphEntry`] × glyph_count (sorted by codepoint), then [`KernEntry`] ×
//!   kern_count at `kern_offset` (sorted by left, then right), then 8-bit
//!   coverage bitmaps. Offsets inside a font section are relative to the
//!   section start. Sizes are in panel pixels, fixed point with 6 fractional
//!   bits (`_q6`).

use crate::types::FACE_COUNT;

pub const PACK_MAGIC: [u8; 4] = *b"SMKB";
/// v2: section table (fonts + clips). v1 was a bare clip table.
pub const PACK_VERSION: u16 = 2;

/// Size of the external QSPI flash reserved for the asset pack.
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
pub const SECTION_ENTRY_LEN: usize = 16;
pub const CLIP_ENTRY_LEN: usize = 16;
pub const CLIPS_HEADER_LEN: usize = 4;
pub const FONT_HEADER_LEN: usize = 24;
pub const GLYPH_ENTRY_LEN: usize = 16;
pub const KERN_ENTRY_LEN: usize = 8;

/// Fixed-point scale for font metrics: 1/64 panel pixel.
pub const Q6: f32 = 64.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum SectionKind {
    Clips = 1,
    Font = 2,
}

impl SectionKind {
    pub const fn from_u16(v: u16) -> Option<SectionKind> {
        match v {
            1 => Some(SectionKind::Clips),
            2 => Some(SectionKind::Font),
            _ => None,
        }
    }
}

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

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn i16_at(b: &[u8], i: usize) -> i16 {
    i16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackHeader {
    pub version: u16,
    pub section_count: u16,
    /// Total pack length in bytes, header included.
    pub total_len: u32,
}

impl PackHeader {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..4].copy_from_slice(&PACK_MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.section_count.to_le_bytes());
        b[8..12].copy_from_slice(&self.total_len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; HEADER_LEN]) -> Option<PackHeader> {
        if b[0..4] != PACK_MAGIC {
            return None;
        }
        Some(PackHeader {
            version: u16_at(b, 4),
            section_count: u16_at(b, 6),
            total_len: u32_at(b, 8),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionEntry {
    pub kind: u16,
    /// Kind-specific: for fonts, the canvas px size (see [`FontHeader`]).
    pub id: u16,
    /// Absolute offset of the section within the pack.
    pub offset: u32,
    pub len: u32,
}

impl SectionEntry {
    pub fn encode(&self) -> [u8; SECTION_ENTRY_LEN] {
        let mut b = [0u8; SECTION_ENTRY_LEN];
        b[0..2].copy_from_slice(&self.kind.to_le_bytes());
        b[2..4].copy_from_slice(&self.id.to_le_bytes());
        b[4..8].copy_from_slice(&self.offset.to_le_bytes());
        b[8..12].copy_from_slice(&self.len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; SECTION_ENTRY_LEN]) -> SectionEntry {
        SectionEntry {
            kind: u16_at(b, 0),
            id: u16_at(b, 2),
            offset: u32_at(b, 4),
            len: u32_at(b, 8),
        }
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
            id: u16_at(b, 0),
            frame_count: u16_at(b, 2),
            fps: b[4],
            flags: b[5],
            offset: u32_at(b, 8),
        }
    }

    pub fn frame_offset(&self, frame: u16) -> u32 {
        self.offset + frame as u32 * CUBE_FRAME_BYTES as u32
    }

    pub fn looping(&self) -> bool {
        self.flags & Self::FLAG_LOOP != 0
    }
}

/// One font weight at one size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontHeader {
    /// CSS weight (400, 700, …).
    pub weight: u16,
    /// Size in mockup canvas pixels, as the spec gives it (panel px = × 96/166).
    pub canvas_px: u16,
    pub glyph_count: u16,
    pub kern_count: u16,
    /// Font ascent above the baseline, panel px q6 (positive).
    pub ascent_q6: i16,
    /// Font descent below the baseline, panel px q6 (negative).
    pub descent_q6: i16,
    /// Offset of the kerning table, relative to the section start.
    pub kern_offset: u32,
}

impl FontHeader {
    pub fn encode(&self) -> [u8; FONT_HEADER_LEN] {
        let mut b = [0u8; FONT_HEADER_LEN];
        b[0..2].copy_from_slice(&self.weight.to_le_bytes());
        b[2..4].copy_from_slice(&self.canvas_px.to_le_bytes());
        b[4..6].copy_from_slice(&self.glyph_count.to_le_bytes());
        b[6..8].copy_from_slice(&self.kern_count.to_le_bytes());
        b[8..10].copy_from_slice(&self.ascent_q6.to_le_bytes());
        b[10..12].copy_from_slice(&self.descent_q6.to_le_bytes());
        b[12..16].copy_from_slice(&self.kern_offset.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; FONT_HEADER_LEN]) -> FontHeader {
        FontHeader {
            weight: u16_at(b, 0),
            canvas_px: u16_at(b, 2),
            glyph_count: u16_at(b, 4),
            kern_count: u16_at(b, 6),
            ascent_q6: i16_at(b, 8),
            descent_q6: i16_at(b, 10),
            kern_offset: u32_at(b, 12),
        }
    }
}

/// A glyph's metrics and bitmap location. The bitmap is `width × height`
/// bytes of coverage (0–255), top row first; its top-left corner sits at
/// `(pen_x + x_min, baseline - y_max)` with `y_max = y_min + height`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphEntry {
    pub codepoint: u32,
    pub advance_q6: u16,
    pub width: u8,
    pub height: u8,
    /// Left edge of the bitmap relative to the pen, panel px.
    pub x_min: i16,
    /// Bottom edge of the bitmap relative to the baseline, panel px (up is +).
    pub y_min: i16,
    /// Relative to the section start.
    pub bitmap_offset: u32,
}

impl GlyphEntry {
    pub fn encode(&self) -> [u8; GLYPH_ENTRY_LEN] {
        let mut b = [0u8; GLYPH_ENTRY_LEN];
        b[0..4].copy_from_slice(&self.codepoint.to_le_bytes());
        b[4..6].copy_from_slice(&self.advance_q6.to_le_bytes());
        b[6] = self.width;
        b[7] = self.height;
        b[8..10].copy_from_slice(&self.x_min.to_le_bytes());
        b[10..12].copy_from_slice(&self.y_min.to_le_bytes());
        b[12..16].copy_from_slice(&self.bitmap_offset.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; GLYPH_ENTRY_LEN]) -> GlyphEntry {
        GlyphEntry {
            codepoint: u32_at(b, 0),
            advance_q6: u16_at(b, 4),
            width: b[6],
            height: b[7],
            x_min: i16_at(b, 8),
            y_min: i16_at(b, 10),
            bitmap_offset: u32_at(b, 12),
        }
    }
}

/// Pair kerning: the pen moves by `adjust_q6` between `left` and `right`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernEntry {
    pub left: u16,
    pub right: u16,
    pub adjust_q6: i16,
}

impl KernEntry {
    pub fn encode(&self) -> [u8; KERN_ENTRY_LEN] {
        let mut b = [0u8; KERN_ENTRY_LEN];
        b[0..2].copy_from_slice(&self.left.to_le_bytes());
        b[2..4].copy_from_slice(&self.right.to_le_bytes());
        b[4..6].copy_from_slice(&self.adjust_q6.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; KERN_ENTRY_LEN]) -> KernEntry {
        KernEntry {
            left: u16_at(b, 0),
            right: u16_at(b, 2),
            adjust_q6: i16_at(b, 4),
        }
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
            section_count: 3,
            total_len: 99,
        };
        assert_eq!(PackHeader::decode(&h.encode()), Some(h));
        let s = SectionEntry {
            kind: SectionKind::Font as u16,
            id: 50,
            offset: 64,
            len: 1234,
        };
        assert_eq!(SectionEntry::decode(&s.encode()), s);
        let c = ClipEntry {
            id: 2,
            frame_count: 40,
            fps: 30,
            flags: ClipEntry::FLAG_LOOP,
            offset: 64,
        };
        assert_eq!(ClipEntry::decode(&c.encode()), c);
        let f = FontHeader {
            weight: 700,
            canvas_px: 94,
            glyph_count: 97,
            kern_count: 12,
            ascent_q6: 3000,
            descent_q6: -900,
            kern_offset: 1600,
        };
        assert_eq!(FontHeader::decode(&f.encode()), f);
        let g = GlyphEntry {
            codepoint: 'd' as u32,
            advance_q6: 1500,
            width: 20,
            height: 30,
            x_min: -1,
            y_min: -8,
            bitmap_offset: 4000,
        };
        assert_eq!(GlyphEntry::decode(&g.encode()), g);
        let k = KernEntry {
            left: 'A' as u16,
            right: 'V' as u16,
            adjust_q6: -40,
        };
        assert_eq!(KernEntry::decode(&k.encode()), k);
    }
}
