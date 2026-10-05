//! Framebuffers and the panel output pipeline (SIM_SPEC B1).
//!
//! Content is drawn into a framebuffer per face, in the target's pixel type
//! ([`smokebomb_hal::Target`]): 8-bit grey on the 96×96 die (0 = off, 255 =
//! full, the mockup's canvas range), RGB565 on the 64×64 one. Each target
//! then packs it for its panel ([`crate::target::DisplayTarget::pack`]):
//! [`Framebuffer::quantize`] reduces grey to the SSD1317's 16 levels with the
//! mockup's ordered dither at 4 bits per pixel; RGB565 goes out two bytes a
//! pixel, high byte first. Six grey buffers are 54 KB, six RGB565 ones 48 KB.
//!
//! Screens draw into these through [`crate::gfx`]; the built-in 3x5 digits
//! below remain only for the placeholder menu and Nest screens.

use smokebomb_hal::{FrameBytes, Grey96, Pixel, Target, PANEL_HEIGHT, PANEL_WIDTH};

use crate::orientation::Quarter;

/// Pixels in a 96×96 face ([`Grey96`]).
pub const PIXELS: usize = PANEL_WIDTH * PANEL_HEIGHT;

/// Foreground white `#F4F5F7`; the quantiser uses the brightest channel.
pub const FG: u8 = 0xF7;
/// Dud grey `#8A8C90`.
pub const DUD: u8 = 0x90;

pub struct Framebuffer<T: Target = Grey96> {
    buf: T::Pixels,
}

impl<T: Target> Clone for Framebuffer<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Target> Copy for Framebuffer<T> {}

impl<T: Target> Default for Framebuffer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Target> Framebuffer<T> {
    pub const WIDTH: usize = T::WIDTH;
    pub const HEIGHT: usize = T::HEIGHT;

    pub const fn new() -> Self {
        Self { buf: T::BLANK_PIXELS }
    }

    pub fn pixels(&self) -> &[T::Pixel] {
        self.buf.as_ref()
    }

    pub fn clear(&mut self) {
        self.buf.as_mut().fill(T::Pixel::default());
    }

    pub fn pixel(&self, x: usize, y: usize) -> T::Pixel {
        self.buf.as_ref()[y * T::WIDTH + x]
    }

    /// Set a pixel. Out-of-range coordinates are ignored.
    pub fn put(&mut self, x: usize, y: usize, p: T::Pixel) {
        if x < T::WIDTH && y < T::HEIGHT {
            self.buf.as_mut()[y * T::WIDTH + x] = p;
        }
    }

    /// Dim every pixel to `factor` of its level (0–1): the Nest's dimmed
    /// screens.
    pub fn scale(&mut self, factor: f32) {
        if factor >= 1.0 {
            return;
        }
        for p in self.buf.as_mut().iter_mut() {
            p.scale(factor);
        }
    }

    /// Additive blend (canvas "lighter") of a grey level, saturating.
    pub fn add_pixel(&mut self, x: usize, y: usize, value: u8) {
        self.add_color(x, y, smokebomb_hal::Color::grey(value));
    }

    /// Additive blend of a colour, saturating per channel.
    pub fn add_color(&mut self, x: usize, y: usize, c: smokebomb_hal::Color) {
        if x < T::WIDTH && y < T::HEIGHT {
            self.buf.as_mut()[y * T::WIDTH + x].add(c);
        }
    }

    /// Source-over blend of grey `value` at opacity `alpha` (0–1).
    pub fn blend(&mut self, x: usize, y: usize, value: f32, alpha: f32) {
        self.blend_rgb(x, y, [value, value, value], alpha);
    }

    /// Source-over blend of a colour (0–255 a channel) at `alpha`.
    #[inline]
    pub fn blend_rgb(&mut self, x: usize, y: usize, c: [f32; 3], alpha: f32) {
        self.buf.as_mut()[y * T::WIDTH + x].blend(c, alpha);
    }

    /// Fill a rectangle given in content coordinates, rotated by `rot` about
    /// the panel centre.
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, value: u8, rot: Quarter) {
        let p = {
            let mut p = T::Pixel::default();
            p.blend([value as f32; 3], 1.0);
            p
        };
        for yy in y..y + h {
            for xx in x..x + w {
                if xx < T::WIDTH && yy < T::HEIGHT {
                    let (px, py) = rot.map_in(xx, yy, T::WIDTH);
                    self.put(px, py, p);
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
        let x0 = T::WIDTH.saturating_sub(total_w) / 2;
        let y0 = T::HEIGHT.saturating_sub(5 * scale) / 2;
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
}

impl Framebuffer<Grey96> {
    /// Set a grey pixel. Out-of-range coordinates are ignored.
    pub fn set_pixel(&mut self, x: usize, y: usize, value: u8) {
        self.put(x, y, value);
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

impl Framebuffer<smokebomb_hal::Rgb64> {
    /// Pack for the panel: two bytes a pixel, row-major, high byte first
    /// (SSD1357 datasheet Rev 1.0, Table 6-7, 8-bit/serial 65k: the first
    /// byte carries C4–C0 B5–B3, the second B2–B0 A4–A0. With colour C wired
    /// to red and A to blue that is RGB565 high byte first; which sub-pixel
    /// is red is set by the panel's wiring and the colour-swap remap bit,
    /// still to confirm on the module).
    pub fn pack565(&self, out: &mut [u8; 64 * 64 * 2]) {
        for (p, o) in self.buf.iter().zip(out.chunks_exact_mut(2)) {
            o.copy_from_slice(&p.to_be_bytes());
        }
    }

    /// Replace the contents with a packed RGB565 frame.
    pub fn load_packed565(&mut self, packed: &[u8; 64 * 64 * 2]) {
        for (p, b) in self.buf.iter_mut().zip(packed.chunks_exact(2)) {
            *p = smokebomb_hal::Rgb565::from_be_bytes([b[0], b[1]]);
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
    use smokebomb_hal::{Color, Rgb565, Rgb64};

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
        let mut fb = Framebuffer::<Grey96>::new();
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
        let mut fb = Framebuffer::<Grey96>::new();
        fb.load_packed(&packed);
        assert_eq!((fb.pixel(0, 0), fb.pixel(1, 0)), (170, 85));
    }

    #[test]
    fn number_is_drawn() {
        let mut fb = Framebuffer::<Grey96>::new();
        fb.draw_number(100, 6, FG, Quarter::R0);
        assert!(fb.pixels().iter().any(|&b| b != 0));
        let mut fb = Framebuffer::<Rgb64>::new();
        fb.draw_number(100, 4, FG, Quarter::R90);
        assert!(fb.pixels().iter().any(|&b| b != Rgb565::BLACK));
    }

    #[test]
    fn rgb565_frame_is_row_major_high_byte_first() {
        let mut fb = Framebuffer::<Rgb64>::new();
        fb.put(0, 0, Rgb565::from_rgb(255, 0, 0));
        fb.put(1, 0, Rgb565::from_rgb(0, 0, 255));
        fb.put(0, 1, Rgb565::from_rgb(0, 255, 0));
        let mut out = [0u8; 8192];
        fb.pack565(&mut out);
        assert_eq!(&out[0..4], &[0xF8, 0x00, 0x00, 0x1F]);
        assert_eq!(&out[128..130], &[0x07, 0xE0]);
        let mut back = Framebuffer::<Rgb64>::new();
        back.load_packed565(&out);
        assert_eq!(back.pixels(), fb.pixels());
    }

    #[test]
    fn colour_blends_and_adds_on_both_targets() {
        let mut g = Framebuffer::<Grey96>::new();
        g.blend_rgb(3, 3, Color::hex(0x8A8C90).to_f32(), 1.0);
        assert_eq!(g.pixel(3, 3), DUD);
        let mut c = Framebuffer::<Rgb64>::new();
        c.blend_rgb(3, 3, Color::hex(0xF5C451).to_f32(), 1.0);
        assert_eq!(c.pixel(3, 3), Rgb565::from_color(Color::hex(0xF5C451)));
        c.add_color(3, 3, Color::WHITE);
        assert_eq!(c.pixel(3, 3).color(), Color::WHITE);
    }
}
