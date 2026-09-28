//! 4bpp framebuffers and minimal text rendering for the 96x96 panels.
//!
//! Six framebuffers are 27 KB total, well inside the nRF54L15's 256 KB RAM.
//! Real typography (bitmap Space Grotesk from the asset pack) replaces the
//! built-in 3x5 digits later.

use smokebomb_hal::{Face, FrameBytes, FACE_COUNT, FRAME_BYTES, PANEL_HEIGHT, PANEL_WIDTH};
use smokebomb_shared::RollRecord;

#[derive(Clone, Copy)]
pub struct Framebuffer {
    buf: FrameBytes,
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framebuffer {
    pub const fn new() -> Self {
        Self {
            buf: [0; FRAME_BYTES],
        }
    }

    pub fn bytes(&self) -> &FrameBytes {
        &self.buf
    }

    pub fn bytes_mut(&mut self) -> &mut FrameBytes {
        &mut self.buf
    }

    pub fn clear(&mut self) {
        self.buf.fill(0);
    }

    /// Set a pixel to a 4-bit gray `level` (0 = off, 15 = full). Out-of-range
    /// coordinates are ignored.
    pub fn set_pixel(&mut self, x: usize, y: usize, level: u8) {
        if x >= PANEL_WIDTH || y >= PANEL_HEIGHT {
            return;
        }
        let i = y * PANEL_WIDTH + x;
        let byte = &mut self.buf[i / 2];
        let level = level & 0x0f;
        if i % 2 == 0 {
            *byte = (*byte & 0x0f) | (level << 4);
        } else {
            *byte = (*byte & 0xf0) | level;
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        let i = y * PANEL_WIDTH + x;
        let byte = self.buf[i / 2];
        if i % 2 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        }
    }

    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, level: u8) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set_pixel(xx, yy, level);
            }
        }
    }

    /// Draw `n` centred on the panel with the built-in digit font, each font
    /// pixel scaled to `scale` x `scale` panel pixels.
    pub fn draw_number(&mut self, n: u16, scale: usize) {
        let mut digits = [0u8; 5];
        let mut len = 0;
        let mut v = n;
        loop {
            digits[len] = (v % 10) as u8;
            len += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        let glyph_w = 3 * scale;
        let gap = scale;
        let total_w = len * glyph_w + (len - 1) * gap;
        let x0 = PANEL_WIDTH.saturating_sub(total_w) / 2;
        let y0 = PANEL_HEIGHT.saturating_sub(5 * scale) / 2;
        for i in 0..len {
            let d = digits[len - 1 - i];
            self.draw_glyph(&DIGITS[d as usize], x0 + i * (glyph_w + gap), y0, scale, 15);
        }
    }

    fn draw_glyph(&mut self, rows: &[u8; 5], x: usize, y: usize, scale: usize, level: u8) {
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    self.fill_rect(x + col * scale, y + row * scale, scale, scale, level);
                }
            }
        }
    }
}

/// 3x5 digit bitmaps, MSB = leftmost column.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Result layout: the face pointing up shows the total, the four side faces
/// show individual dice, the face on the table stays dark.
pub fn draw_result(frames: &mut [Framebuffer; FACE_COUNT], up: Face, record: &RollRecord) {
    frames[up.index()].draw_number(record.total(), 6);
    if record.values.len() > 1 {
        let sides = Face::ALL.into_iter().filter(|f| *f != up && *f != up.opposite());
        for (face, value) in sides.zip(record.values.iter()) {
            frames[face.index()].draw_number(*value as u16, 4);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_pack_two_per_byte() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(0, 0, 0xA);
        fb.set_pixel(1, 0, 0x5);
        assert_eq!(fb.bytes()[0], 0xA5);
        assert_eq!(fb.pixel(0, 0), 0xA);
        assert_eq!(fb.pixel(1, 0), 0x5);
    }

    #[test]
    fn number_is_drawn() {
        let mut fb = Framebuffer::new();
        fb.draw_number(100, 6);
        assert!(fb.bytes().iter().any(|&b| b != 0));
    }
}
