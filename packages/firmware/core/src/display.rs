//! Framebuffers and the panel output pipeline (SIM_SPEC B1).
//!
//! Content is drawn into an 8-bit grey buffer per face (0 = off, 255 = full),
//! the same value range as the mockup's canvas. [`Framebuffer::quantize`] then
//! reduces it to the SSD1317's 16 levels with the mockup's ordered dither and
//! packs it at 4 bits per pixel for the panel. Six buffers are 54 KB, within
//! the nRF54L15's 256 KB of RAM.
//!
//! Screens draw into these through [`crate::gfx`]; the built-in 3x5 digits
//! below remain only for the placeholder menu and Nest screens.

use smokebomb_hal::{FrameBytes, PANEL_HEIGHT, PANEL_WIDTH};

use crate::orientation::Quarter;

pub const PIXELS: usize = PANEL_WIDTH * PANEL_HEIGHT;

/// Foreground white `#F4F5F7`; the quantiser uses the brightest channel.
pub const FG: u8 = 0xF7;
/// Dud grey `#8A8C90`.
pub const DUD: u8 = 0x90;

#[derive(Clone, Copy)]
pub struct Framebuffer {
    buf: [u8; PIXELS],
}

impl Default for Framebuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Framebuffer {
    pub const fn new() -> Self {
        Self { buf: [0; PIXELS] }
    }

    pub fn pixels(&self) -> &[u8; PIXELS] {
        &self.buf
    }

    pub fn clear(&mut self) {
        self.buf.fill(0);
    }

    /// Set a pixel. Out-of-range coordinates are ignored.
    pub fn set_pixel(&mut self, x: usize, y: usize, value: u8) {
        if x < PANEL_WIDTH && y < PANEL_HEIGHT {
            self.buf[y * PANEL_WIDTH + x] = value;
        }
    }

    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        self.buf[y * PANEL_WIDTH + x]
    }

    /// Additive blend (canvas "lighter"), saturating at 255.
    pub fn add_pixel(&mut self, x: usize, y: usize, value: u8) {
        if x < PANEL_WIDTH && y < PANEL_HEIGHT {
            let p = &mut self.buf[y * PANEL_WIDTH + x];
            *p = p.saturating_add(value);
        }
    }

    /// Source-over blend of `value` at opacity `alpha` (0–1).
    pub fn blend(&mut self, x: usize, y: usize, value: f32, alpha: f32) {
        let p = &mut self.buf[y * PANEL_WIDTH + x];
        let a = alpha.clamp(0.0, 1.0);
        *p = (*p as f32 + (value - *p as f32) * a + 0.5) as u8;
    }

    /// Add a packed 4bpp frame on top (level `l` → `l * 17`), saturating.
    pub fn add_packed(&mut self, packed: &FrameBytes) {
        for (i, px) in self.buf.iter_mut().enumerate() {
            let b = packed[i / 2];
            let level = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
            *px = px.saturating_add(level * 17);
        }
    }

    /// Fill a rectangle given in content coordinates, rotated by `rot` about
    /// the panel centre.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, value: u8, rot: Quarter) {
        for yy in y..y + h {
            for xx in x..x + w {
                if xx < PANEL_WIDTH && yy < PANEL_HEIGHT {
                    let (px, py) = rot.map(xx, yy);
                    self.set_pixel(px, py, value);
                }
            }
        }
    }

    /// Draw `n` centred on the panel with the built-in digit font, each font
    /// pixel scaled to `scale` x `scale` panel pixels.
    pub fn draw_number(&mut self, n: u16, scale: usize, value: u8, rot: Quarter) {
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
            let x = x0 + i * (glyph_w + gap);
            for (row, bits) in DIGITS[d as usize].iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        self.fill_rect(x + col * scale, y0 + row * scale, scale, scale, value, rot);
                    }
                }
            }
        }
    }

    /// Replace the contents with a packed 4bpp frame (level `l` → `l * 17`).
    pub fn load_packed(&mut self, packed: &FrameBytes) {
        for (i, px) in self.buf.iter_mut().enumerate() {
            let b = packed[i / 2];
            let level = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
            *px = level * 17;
        }
    }

    /// Reduce to 16 levels and pack for the panel, exactly as the mockup does:
    /// `level = min(15, floor(v / 255 * 15 + D[x, y]))`, high nibble first.
    pub fn quantize(&self, out: &mut FrameBytes) {
        for (i, (&v, &o)) in self.buf.iter().zip(DITHER_OFFSET.iter()).enumerate() {
            let level = quantize_pixel(v, o);
            let b = &mut out[i / 2];
            if i % 2 == 0 {
                *b = (*b & 0x0f) | (level << 4);
            } else {
                *b = (*b & 0xf0) | level;
            }
        }
    }
}

/// `v / 255 * 15` is `v / 17`, so `floor(v / 17 + D)` equals
/// `floor((v + floor(17 D)) / 17)` for integer `v`: no floats per pixel.
#[inline]
fn quantize_pixel(v: u8, offset: u8) -> u8 {
    ((v as u16 + offset as u16) / 17).min(15) as u8
}

/// `floor(17 * D[x, y])` for the mockup's ordered dither
/// `D = 0.02 + 0.96 * frac(0.7548776662 (x + 1) + 0.5698402910 (y + 1))`.
/// `D` is rounded to f32, as the mockup stores it in a `Float32Array`.
pub static DITHER_OFFSET: [u8; PIXELS] = dither_offsets();

const fn dither_offsets() -> [u8; PIXELS] {
    let mut out = [0u8; PIXELS];
    let mut i = 0;
    while i < PIXELS {
        let x = (i % PANEL_WIDTH) as f64;
        let y = (i / PANEL_WIDTH) as f64;
        let v = 0.7548776662 * (x + 1.0) + 0.5698402910 * (y + 1.0);
        let frac = v - (v as u64) as f64;
        let d = (0.02 + frac * 0.96) as f32 as f64;
        out[i] = (d * 17.0) as u8;
        i += 1;
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The mockup's formula, evaluated the way the browser does it.
    fn reference_level(v: u8, x: usize, y: usize) -> u8 {
        let frac = (0.7548776662 * (x as f64 + 1.0) + 0.5698402910 * (y as f64 + 1.0)) % 1.0;
        let d = (0.02 + frac * 0.96) as f32 as f64;
        ((v as f64 / 255.0 * 15.0 + d).floor() as u8).min(15)
    }

    #[test]
    fn quantizer_matches_the_mockup_exactly() {
        let mut mismatches = 0;
        for y in 0..PANEL_HEIGHT {
            for x in 0..PANEL_WIDTH {
                let o = DITHER_OFFSET[y * PANEL_WIDTH + x];
                for v in 0..=255u8 {
                    if quantize_pixel(v, o) != reference_level(v, x, y) {
                        mismatches += 1;
                    }
                }
            }
        }
        assert_eq!(mismatches, 0);
    }

    #[test]
    fn quantize_packs_high_nibble_first() {
        let mut fb = Framebuffer::new();
        fb.set_pixel(0, 0, 255);
        let mut out = [0u8; smokebomb_hal::FRAME_BYTES];
        fb.quantize(&mut out);
        assert_eq!(out[0] >> 4, 15);
        assert_eq!(out[0] & 0x0f, 0);
    }

    #[test]
    fn load_packed_round_trips_levels() {
        let mut packed = [0u8; smokebomb_hal::FRAME_BYTES];
        packed[0] = 0xA5;
        let mut fb = Framebuffer::new();
        fb.load_packed(&packed);
        assert_eq!((fb.pixel(0, 0), fb.pixel(1, 0)), (170, 85));
    }

    #[test]
    fn number_is_drawn() {
        let mut fb = Framebuffer::new();
        fb.draw_number(100, 6, FG, Quarter::R0);
        assert!(fb.pixels().iter().any(|&b| b != 0));
    }
}
