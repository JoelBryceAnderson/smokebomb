//! Display targets: which panel the faces carry, chosen at compile time.
//!
//! Two exist:
//!
//! * [`Grey96`] (`TARGET_34_GREY96`): the 34 mm die, six SSD1317 96×96
//!   panels, 16 grey levels. The firmware's original target; its output is
//!   pinned by the screen snapshots.
//! * [`Rgb64`] (`TARGET_30_RGB64`): the 30 mm proof of concept, six 0.6"
//!   64×64 RGB PMOLEDs (SSD1357 class), RGB565.
//!
//! A target fixes the panel's size, its pixel type, the packed bytes a frame
//! goes to the panel as, and its physical size. Drawing code is generic over
//! the target and monomorphised, so a board image carries exactly one and
//! pays nothing for the other. The simulator builds both and picks one at
//! launch.
//!
//! Pixels are blended in a target-independent colour ([`Color`], or `[f32;
//! 3]` while blending); each pixel type reduces it the way its panel does:
//! grey takes the brightest channel (the mockup's quantiser, SIM_SPEC B1),
//! RGB565 keeps the top 5/6/5 bits.

/// A 24-bit colour, the way screens name colours. Grey targets show the
/// brightest channel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color::grey(0);
    pub const WHITE: Color = Color::grey(255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b }
    }

    pub const fn grey(v: u8) -> Color {
        Color { r: v, g: v, b: v }
    }

    /// From `0xRRGGBB`.
    pub const fn hex(v: u32) -> Color {
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// The brightest channel: what a grey panel shows.
    pub const fn max_channel(self) -> u8 {
        let m = if self.r > self.g { self.r } else { self.g };
        if m > self.b {
            m
        } else {
            self.b
        }
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.r as f32, self.g as f32, self.b as f32]
    }

    /// Grey level `v` (0–255, unrounded) tinted by this colour. White
    /// returns `v` itself in every channel, exactly.
    pub fn tint(self, v: f32) -> [f32; 3] {
        if self == Color::WHITE {
            return [v, v, v];
        }
        [
            v * self.r as f32 / 255.0,
            v * self.g as f32 / 255.0,
            v * self.b as f32 / 255.0,
        ]
    }
}

/// One stored pixel. Blending happens in 0–255 floats per channel; each type
/// rounds the result into what it stores.
pub trait Pixel: Copy + Default + PartialEq + core::fmt::Debug + 'static {
    /// Source-over: move toward `c` by `alpha` (0–1).
    fn blend(&mut self, c: [f32; 3], alpha: f32);
    /// Additive (canvas "lighter"), saturating.
    fn add(&mut self, c: Color);
    /// Dim to `factor` (0–1) of its brightness.
    fn scale(&mut self, factor: f32);
    /// The pixel as a 24-bit colour (grey reads back as grey).
    fn color(self) -> Color;
    /// Grey level `v`, exactly.
    fn grey(v: u8) -> Self;
    /// The pixel's brightness, the way a grey panel would show it (0–255).
    fn level(self) -> u8 {
        self.color().max_channel()
    }
}

/// 8-bit grey (0 = off, 255 = full). The panel shows its top 4 bits after
/// dithering.
impl Pixel for u8 {
    #[inline]
    fn blend(&mut self, c: [f32; 3], alpha: f32) {
        let value = c[0].max(c[1]).max(c[2]);
        let a = alpha.clamp(0.0, 1.0);
        *self = (*self as f32 + (value - *self as f32) * a + 0.5) as u8;
    }

    #[inline]
    fn add(&mut self, c: Color) {
        *self = self.saturating_add(c.max_channel());
    }

    #[inline]
    fn scale(&mut self, factor: f32) {
        *self = (*self as f32 * factor + 0.5) as u8;
    }

    #[inline]
    fn color(self) -> Color {
        Color::grey(self)
    }

    #[inline]
    fn grey(v: u8) -> Self {
        v
    }
}

/// RGB565: red in the top 5 bits, green the middle 6, blue the low 5.
/// Stored as a native `u16`; [`Rgb565::to_be_bytes`] gives the order the
/// panel expects on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb565(pub u16);

impl Rgb565 {
    pub const BLACK: Rgb565 = Rgb565(0);

    /// Keep the top 5/6/5 bits, as the mockup's `& 0xF8, & 0xFC, & 0xF8`.
    pub const fn from_rgb(r: u8, g: u8, b: u8) -> Rgb565 {
        Rgb565(((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3))
    }

    pub const fn from_color(c: Color) -> Rgb565 {
        Rgb565::from_rgb(c.r, c.g, c.b)
    }

    /// Expanded back to 8 bits a channel, replicating the top bits into the
    /// bottom so full scale stays 255.
    pub const fn to_color(self) -> Color {
        let r = ((self.0 >> 11) & 0x1f) as u8;
        let g = ((self.0 >> 5) & 0x3f) as u8;
        let b = (self.0 & 0x1f) as u8;
        Color::rgb((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))
    }

    /// High byte first: the order an SSD1357 takes 65k-colour data over SPI
    /// in its default (non-swapped) mode. **Datasheet TODO:** confirm the
    /// byte order and the RGB/BGR setting against the SSD1357 remap command
    /// once we have the datasheet; flipping either is a one-line change.
    pub const fn to_be_bytes(self) -> [u8; 2] {
        self.0.to_be_bytes()
    }

    pub const fn from_be_bytes(b: [u8; 2]) -> Rgb565 {
        Rgb565(u16::from_be_bytes(b))
    }
}

impl Pixel for Rgb565 {
    #[inline]
    fn blend(&mut self, c: [f32; 3], alpha: f32) {
        let a = alpha.clamp(0.0, 1.0);
        let old = self.to_color().to_f32();
        let mix = |o: f32, n: f32| (o + (n - o) * a + 0.5).clamp(0.0, 255.0) as u8;
        *self = Rgb565::from_rgb(mix(old[0], c[0]), mix(old[1], c[1]), mix(old[2], c[2]));
    }

    #[inline]
    fn add(&mut self, c: Color) {
        let o = self.to_color();
        *self = Rgb565::from_rgb(
            o.r.saturating_add(c.r),
            o.g.saturating_add(c.g),
            o.b.saturating_add(c.b),
        );
    }

    #[inline]
    fn scale(&mut self, factor: f32) {
        let o = self.to_color().to_f32();
        let s = |v: f32| (v * factor + 0.5) as u8;
        *self = Rgb565::from_rgb(s(o[0]), s(o[1]), s(o[2]));
    }

    #[inline]
    fn color(self) -> Color {
        self.to_color()
    }

    #[inline]
    fn grey(v: u8) -> Self {
        Rgb565::from_color(Color::grey(v))
    }
}

/// Which target, for anything that has to say so at run time (the
/// simulator's frame packets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TargetId {
    /// 34 mm die, 96×96, 16 grey levels, packed 4 bpp.
    Grey96 = 1,
    /// 30 mm die, 64×64, RGB565, 2 bytes a pixel, high byte first.
    Rgb64 = 2,
}

/// How a face's panel is turned in its window, relative to the face's
/// drawing axes (`orientation::BASES` in the core), seen from outside the
/// die. The firmware draws every face in its drawing axes; a turned panel
/// gets its frame turned back as it's packed, so the picture on the glass
/// is the same whichever way the panel went in.
///
/// A panel's own axes: its columns run toward the end its ribbon leaves
/// from, its rows across that, the face's outward normal completing them
/// (so the panel's "up" is the normal crossed with the ribbon direction).
/// **Datasheet TODO:** that's the module drawing's convention, not yet
/// checked against the panel's scan order; if the panel scans the other way
/// round, every face of a target turns by the same extra amount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mount {
    /// The panel's axes are the face's drawing axes.
    #[default]
    Upright,
    /// Turned a quarter clockwise: the panel's right is the face's down.
    Clockwise,
    /// Turned a half turn.
    UpsideDown,
    /// Turned a quarter anticlockwise: the panel's right is the face's up.
    Anticlockwise,
}

impl Mount {
    /// For panel pixel (`col`, `row`) of a `side`-pixel square panel, the
    /// pixel of the face's upright picture that lands there.
    #[inline]
    pub const fn source(self, col: usize, row: usize, side: usize) -> (usize, usize) {
        let m = side - 1;
        match self {
            Mount::Upright => (col, row),
            Mount::Clockwise => (m - row, col),
            Mount::UpsideDown => (m - col, m - row),
            Mount::Anticlockwise => (row, m - col),
        }
    }
}

/// A display target. See the module docs.
pub trait Target: Sized + 'static {
    const ID: TargetId;
    /// Panel pixels.
    const WIDTH: usize;
    const HEIGHT: usize;
    const PIXELS: usize = Self::WIDTH * Self::HEIGHT;
    /// Outer size of the die, mm.
    const DIE_MM: f32;
    /// Lit (active) area of the panel, mm square.
    const LIT_MM: f32;
    /// The cover glass's ink mask rounds the lit area with this radius, in
    /// panel pixels. The panel itself is square; the simulator's glass
    /// applies the mask (SIM_SPEC B1 step 4).
    const MASK_RADIUS_PX: f32;
    /// How each face's panel is mounted, in `Face` order.
    const MOUNT: [Mount; crate::FACE_COUNT] = [Mount::Upright; crate::FACE_COUNT];

    /// What a framebuffer stores per pixel.
    type Pixel: Pixel;
    /// One face's pixels, `[Pixel; PIXELS]`.
    type Pixels: Copy + AsRef<[Self::Pixel]> + AsMut<[Self::Pixel]> + 'static;
    /// A byte per pixel: drawing scratch (coverage, blur).
    type Plane: Copy + AsRef<[u8]> + AsMut<[u8]> + 'static;
    /// One frame packed the way the panel takes it.
    type Panel: Copy + AsRef<[u8]> + AsMut<[u8]> + 'static;

    const BLANK_PIXELS: Self::Pixels;
    const BLANK_PLANE: Self::Plane;
    const BLANK_PANEL: Self::Panel;
}

/// `TARGET_34_GREY96`: the 34 mm die with six SSD1317 96×96 panels.
pub struct Grey96;

/// `TARGET_30_RGB64`: the 30 mm die with six 0.6" 64×64 RGB panels.
pub struct Rgb64;

impl Target for Grey96 {
    const ID: TargetId = TargetId::Grey96;
    const WIDTH: usize = 96;
    const HEIGHT: usize = 96;
    const DIE_MM: f32 = 34.0;
    /// ER-OLED0.96-6W active area (SIM_SPEC A2).
    const LIT_MM: f32 = 17.26;
    /// 2.52 mm (SIM_SPEC A2).
    const MASK_RADIUS_PX: f32 = 14.0;

    type Pixel = u8;
    type Pixels = [u8; 96 * 96];
    type Plane = [u8; 96 * 96];
    type Panel = [u8; 96 * 96 / 2];

    const BLANK_PIXELS: Self::Pixels = [0; 96 * 96];
    const BLANK_PLANE: Self::Plane = [0; 96 * 96];
    const BLANK_PANEL: Self::Panel = [0; 96 * 96 / 2];
}

impl Target for Rgb64 {
    const ID: TargetId = TargetId::Rgb64;
    const WIDTH: usize = 64;
    const HEIGHT: usize = 64;
    const DIE_MM: f32 = 30.0;
    /// **Estimate**, from the brief and the mockup's 30 mm option: 10.75 mm
    /// square (≈0.168 mm pitch). Confirm against the NHD-0.6-6464G drawing.
    const LIT_MM: f32 = 10.75;
    /// **Estimate**: the mockup rounds the 30 mm lit area by 34 of its 203
    /// texels, ≈1.80 mm, ≈10.7 panel px. The window's own radius is 1.84 mm.
    const MASK_RADIUS_PX: f32 = 10.7;
    /// No pinwheel (4 Oct): the four side screens' ribbons point down,
    /// toward the lid; the top screen's points to +X, folding straight into
    /// the harness channel; the lid screen's to −X, opposite it. Each panel
    /// sits ~1.8 mm off centre toward its ribbon, so top and lid cancel (as
    /// the four sides do), the lid's fold clears its locating pin at −Z, and
    /// its connector goes on the board's −X side, away from the +X hub.
    /// Against the faces' drawing axes that leaves the top panel upright,
    /// the lid's upside down and the four sides turned a quarter clockwise.
    const MOUNT: [Mount; crate::FACE_COUNT] = [
        Mount::Clockwise,  // +X: ribbon to −Y
        Mount::Clockwise,  // −X: ribbon to −Y
        Mount::Upright,    // +Y: ribbon to +X
        Mount::UpsideDown, // −Y: ribbon to −X
        Mount::Clockwise,  // +Z: ribbon to −Y
        Mount::Clockwise,  // −Z: ribbon to −Y
    ];

    type Pixel = Rgb565;
    type Pixels = [Rgb565; 64 * 64];
    type Plane = [u8; 64 * 64];
    type Panel = [u8; 64 * 64 * 2];

    const BLANK_PIXELS: Self::Pixels = [Rgb565::BLACK; 64 * 64];
    const BLANK_PLANE: Self::Plane = [0; 64 * 64];
    const BLANK_PANEL: Self::Panel = [0; 64 * 64 * 2];
}

/// A rectangle of panel pixels, `[x0, x1) × [y0, y1)`: the part of a frame
/// that changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Region {
    pub x0: u8,
    pub y0: u8,
    pub x1: u8,
    pub y1: u8,
}

impl Region {
    pub const fn full<T: Target>() -> Region {
        Region {
            x0: 0,
            y0: 0,
            x1: T::WIDTH as u8,
            y1: T::HEIGHT as u8,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub const fn width(&self) -> usize {
        (self.x1 - self.x0) as usize
    }

    pub const fn height(&self) -> usize {
        (self.y1 - self.y0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_packs_565_and_goes_out_high_byte_first() {
        let p = Rgb565::from_rgb(0xFF, 0x00, 0x00);
        assert_eq!(p.0, 0xF800);
        assert_eq!(p.to_be_bytes(), [0xF8, 0x00]);
        assert_eq!(Rgb565::from_rgb(0, 0xFF, 0).0, 0x07E0);
        assert_eq!(Rgb565::from_rgb(0, 0, 0xFF).0, 0x001F);
        // The mockup's mask: low bits dropped, not rounded.
        assert_eq!(Rgb565::from_rgb(0x0F, 0x03, 0x07).0, 0x0800);
        assert_eq!(Rgb565::from_be_bytes([0x12, 0x34]).0, 0x1234);
    }

    #[test]
    fn rgb565_round_trips_full_scale_and_its_own_values() {
        assert_eq!(Rgb565::from_color(Color::WHITE).to_color(), Color::WHITE);
        assert_eq!(Rgb565::BLACK.to_color(), Color::BLACK);
        for v in 0..=0xFFFFu16 {
            let p = Rgb565(v);
            assert_eq!(Rgb565::from_color(p.to_color()), p);
        }
    }

    #[test]
    fn grey_blend_is_the_original_formula() {
        for old in [0u8, 17, 128, 255] {
            for v in [0.0f32, 33.3, 247.0, 255.0] {
                for a in [0.0f32, 0.25, 0.85, 1.0] {
                    let mut p = old;
                    p.blend([v, v, v], a);
                    let want = (old as f32 + (v - old as f32) * a + 0.5) as u8;
                    assert_eq!(p, want);
                }
            }
        }
        // A colour on grey shows its brightest channel.
        let mut p = 0u8;
        p.blend(Color::hex(0xF5C451).to_f32(), 1.0);
        assert_eq!(p, 0xF5);
    }

    #[test]
    fn white_tint_is_exact() {
        for v in [0.0f32, 0.1, 150.0, 254.99] {
            assert_eq!(Color::WHITE.tint(v), [v, v, v]);
        }
    }

    #[test]
    fn colour_blends_toward_the_colour() {
        let mut p = Rgb565::BLACK;
        p.blend(Color::rgb(255, 0, 0).to_f32(), 1.0);
        assert_eq!(p, Rgb565::from_rgb(255, 0, 0));
        p.blend(Color::rgb(0, 0, 255).to_f32(), 0.5);
        let c = p.to_color();
        assert!(c.r > 100 && c.r < 140 && c.b > 100 && c.g == 0, "{c:?}");
    }

    #[test]
    fn sizes() {
        assert_eq!(core::mem::size_of::<<Grey96 as Target>::Panel>(), 4608);
        assert_eq!(core::mem::size_of::<<Rgb64 as Target>::Panel>(), 8192);
        assert_eq!(core::mem::size_of::<<Rgb64 as Target>::Pixels>(), 8192);
    }
}
