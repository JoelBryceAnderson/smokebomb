//! Space Grotesk text from the asset pack, and the Pacifico script the
//! Sugarcube wordmark is set in ([`SCRIPT`]).
//!
//! Each font section holds one weight at one size, pre-rasterized at panel
//! resolution with 8-bit coverage (see [`smokebomb_shared::assets`]). Sizes are
//! keyed by the mockup's canvas px, so a screen asks for exactly the size the
//! mockup uses (`700 52px`) and gets that cut.
//!
//! Layout follows the canvas `fillText` the mockup uses: pair kerning, pen
//! advances in sub-pixel units, `textAlign: center` and
//! `textBaseline: middle` (the middle of the ascent–descent box sits on `y`).
//! Glyphs are sampled bilinearly through the painter's transform, so text
//! rotates, scales and slides like any other shape.
//!
//! Glyphs are rasterised at the 96×96 panel's scale ([`K`] panel px per
//! canvas px); on a smaller panel the painter's transform samples them down.
//! The 64×64 screens use their own bitmap fonts instead (`font64`).
//!
//! Hardware note: every glyph is read from the `AssetStore` when drawn. On
//! the nRF54L15 the QSPI flash can be memory-mapped (XIP), which makes that a
//! plain memory read; otherwise add a glyph cache.

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use heapless::Vec;
use libm::floorf;
use smokebomb_hal::{AssetStore, Target, PANEL_WIDTH};
use smokebomb_shared::assets::{
    FontHeader, GlyphEntry, KernEntry, SectionKind, FONT_HEADER_LEN, GLYPH_ENTRY_LEN, KERN_ENTRY_LEN, Q6,
};

use crate::gfx::{Painter, Style, K};
use crate::pack::{PackIndex, MAX_SECTIONS};

/// The weight every screen uses. The mockup asks for 600 for labels but only
/// loads 400/500/700, so it renders them at 700 (SIM_SPEC G14, H8).
pub const BOLD: u16 = 700;
/// The wordmark's script face (Pacifico), which only comes in a few sizes.
pub const SCRIPT: u16 = smokebomb_shared::assets::SCRIPT_WEIGHT;
/// Width of the soft pen edge when script is written on, in panel px.
const PEN_EDGE: f32 = 3.0;

/// Largest glyph bitmap the renderer will draw (bytes).
const GLYPH_BUF: usize = 4096;

#[derive(Clone, Copy, Debug)]
struct FontSize {
    offset: u32,
    header: FontHeader,
}

pub struct Fonts {
    sizes: Vec<FontSize, MAX_SECTIONS>,
    glyph: [u8; GLYPH_BUF],
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            sizes: Vec::new(),
            glyph: [0; GLYPH_BUF],
        }
    }
}

/// Horizontal alignment about `x` (canvas `textAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
}

/// A laid-out glyph: its entry and pen position (panel px, unscaled).
#[derive(Clone, Copy)]
struct Placed {
    entry: GlyphEntry,
    pen: f32,
}

const MAX_TEXT: usize = 32;

impl Fonts {
    /// Read the font table from the pack into `slot`, in place (the glyph
    /// buffer is 4 KB).
    pub fn init<'a, A: AssetStore>(
        slot: &'a mut MaybeUninit<Self>,
        assets: &mut A,
        pack: &PackIndex,
    ) -> &'a mut Self {
        let p = slot.as_mut_ptr();
        // SAFETY: both fields are written through raw pointers before any
        // read, and a zeroed byte array is a valid one. The pattern in
        // `_fields` fails to compile if a field is added and not listed.
        let fonts = unsafe {
            addr_of_mut!((*p).sizes).write(Vec::new());
            addr_of_mut!((*p).glyph).write_bytes(0, 1);
            slot.assume_init_mut()
        };
        #[allow(unused_variables)]
        fn _fields(f: Fonts) {
            let Fonts { sizes, glyph } = f;
        }
        for s in pack.sections(SectionKind::Font) {
            let mut h = [0u8; FONT_HEADER_LEN];
            if assets.read(s.offset, &mut h).is_ok() {
                let _ = fonts.sizes.push(FontSize {
                    offset: s.offset,
                    header: FontHeader::decode(&h),
                });
            }
        }
        fonts
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }

    /// The cut for `weight` at `canvas_px`: exact if present, else the
    /// nearest size of that weight.
    fn pick(&self, weight: u16, canvas_px: u16) -> Option<FontSize> {
        self.sizes
            .iter()
            .filter(|f| f.header.weight == weight)
            .min_by_key(|f| (f.header.canvas_px as i32 - canvas_px as i32).abs())
            .copied()
    }

    /// Like [`Self::pick`], falling back to Space Grotesk when a pack has no
    /// cut of `weight` (a pack built before the script face).
    fn pick_or_bold(&self, weight: u16, canvas_px: u16) -> Option<FontSize> {
        self.pick(weight, canvas_px)
            .or_else(|| self.pick(BOLD, canvas_px))
    }

    fn glyph<A: AssetStore>(assets: &mut A, f: &FontSize, cp: u32) -> Option<GlyphEntry> {
        let (mut lo, mut hi) = (0usize, f.header.glyph_count as usize);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let mut b = [0u8; GLYPH_ENTRY_LEN];
            assets
                .read(
                    f.offset + (FONT_HEADER_LEN + mid * GLYPH_ENTRY_LEN) as u32,
                    &mut b,
                )
                .ok()?;
            let g = GlyphEntry::decode(&b);
            match g.codepoint.cmp(&cp) {
                core::cmp::Ordering::Equal => return Some(g),
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    fn kern<A: AssetStore>(assets: &mut A, f: &FontSize, left: u32, right: u32) -> f32 {
        let (Ok(l), Ok(r)) = (u16::try_from(left), u16::try_from(right)) else {
            return 0.0;
        };
        let (mut lo, mut hi) = (0usize, f.header.kern_count as usize);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let mut b = [0u8; KERN_ENTRY_LEN];
            if assets
                .read(
                    f.offset + f.header.kern_offset + (mid * KERN_ENTRY_LEN) as u32,
                    &mut b,
                )
                .is_err()
            {
                return 0.0;
            }
            let k = KernEntry::decode(&b);
            match (k.left, k.right).cmp(&(l, r)) {
                core::cmp::Ordering::Equal => return k.adjust_q6 as f32 / Q6,
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
            }
        }
        0.0
    }

    /// Lay out `text`; returns the glyphs and the advance width (panel px).
    fn layout<A: AssetStore>(assets: &mut A, f: &FontSize, text: &str) -> (Vec<Placed, MAX_TEXT>, f32) {
        let mut out = Vec::new();
        let mut pen = 0.0;
        let mut prev: Option<u32> = None;
        for ch in text.chars() {
            let cp = ch as u32;
            if let Some(p) = prev {
                pen += Self::kern(assets, f, p, cp);
            }
            if let Some(entry) = Self::glyph(assets, f, cp) {
                let _ = out.push(Placed { entry, pen });
                pen += entry.advance_q6 as f32 / Q6;
            }
            prev = Some(cp);
        }
        (out, pen)
    }

    /// Advance width of `text` in canvas units.
    pub fn measure<A: AssetStore>(&self, assets: &mut A, text: &str, canvas_px: u16) -> f32 {
        self.measure_in(assets, BOLD, text, canvas_px)
    }

    /// Advance width of `text` in `weight` ([`BOLD`] or [`SCRIPT`]), in canvas units.
    pub fn measure_in<A: AssetStore>(&self, assets: &mut A, weight: u16, text: &str, canvas_px: u16) -> f32 {
        match self.pick_or_bold(weight, canvas_px) {
            Some(f) => Self::layout(assets, &f, text).1 / K,
            None => 0.0,
        }
    }

    /// Draw `text` at canvas position `(x, y)` with the mockup's
    /// `font = "700 {canvas_px}px Space Grotesk"`, `textBaseline = middle`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<A: AssetStore, T: Target>(
        &mut self,
        assets: &mut A,
        painter: &mut Painter<T>,
        text: &str,
        x: f32,
        y: f32,
        canvas_px: u16,
        align: Align,
        style: Style,
    ) {
        self.draw_in(assets, painter, BOLD, text, (x, y), canvas_px, align, 1.0, style);
    }

    /// Draw `text` in the wordmark script, centred on `(x, y)` and written
    /// on from the left: only the first `reveal` (0–1) of its width shows,
    /// behind a soft pen edge.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_script<A: AssetStore, T: Target>(
        &mut self,
        assets: &mut A,
        painter: &mut Painter<T>,
        text: &str,
        x: f32,
        y: f32,
        canvas_px: u16,
        reveal: f32,
        style: Style,
    ) {
        self.draw_in(
            assets,
            painter,
            SCRIPT,
            text,
            (x, y),
            canvas_px,
            Align::Center,
            reveal,
            style,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_in<A: AssetStore, T: Target>(
        &mut self,
        assets: &mut A,
        painter: &mut Painter<T>,
        weight: u16,
        text: &str,
        (x, y): (f32, f32),
        canvas_px: u16,
        align: Align,
        reveal: f32,
        style: Style,
    ) {
        if reveal <= 0.0 {
            return;
        }
        let Some(f) = self.pick_or_bold(weight, canvas_px) else {
            return;
        };
        let (glyphs, width) = Self::layout(assets, &f, text);
        // Text space: panel px at the cut's own size, origin at the canvas
        // origin. Canvas units × K = text-space px.
        let start = x * K - if align == Align::Center { width / 2.0 } else { 0.0 };
        // The pen: everything left of it shows, fading over PEN_EDGE.
        let pen = if reveal >= 1.0 {
            f32::INFINITY
        } else {
            start + (width + PEN_EDGE) * reveal
        };
        // `middle` is the middle of the em box, where Chrome scales the
        // font's ascent and descent to span exactly one em.
        let (ascent, descent) = (f.header.ascent_q6 as f32, f.header.descent_q6 as f32);
        let em = f.header.canvas_px as f32 * K;
        let baseline = y * K + em * (ascent + descent) / (2.0 * (ascent - descent));

        painter.begin();
        for g in glyphs {
            let e = g.entry;
            let (w, h) = (e.width as usize, e.height as usize);
            if w == 0 || h == 0 || w * h > GLYPH_BUF {
                continue;
            }
            let buf = &mut self.glyph[..w * h];
            if assets.read(f.offset + e.bitmap_offset, buf).is_err() {
                continue;
            }
            // Bitmap top-left in text space.
            let gx = start + g.pen + e.x_min as f32;
            let gy = baseline - (e.y_min as f32 + h as f32);
            let bounds = painter
                .xf
                .bounds(gx / K, gy / K, (gx + w as f32) / K, (gy + h as f32) / K, 1.0);
            for py in bounds.y0..bounds.y1 {
                for px in bounds.x0..bounds.x1 {
                    let (cx, cy) = painter.xf.inverse(px as f32 + 0.5, py as f32 + 0.5);
                    let mut c = bilinear(buf, w, h, cx * K - gx - 0.5, cy * K - gy - 0.5);
                    if pen.is_finite() {
                        c *= ((pen - cx * K) / PEN_EDGE).clamp(0.0, 1.0);
                    }
                    painter.cover(px, py, c);
                }
            }
            painter.mark(bounds);
        }
        painter.finish(style);
    }
}

/// Sample coverage (0–1) at bitmap coordinates, pixel centres at integers.
fn bilinear(buf: &[u8], w: usize, h: usize, x: f32, y: f32) -> f32 {
    let (x0, y0) = (floorf(x), floorf(y));
    let (fx, fy) = (x - x0, y - y0);
    let at = |ix: f32, iy: f32| -> f32 {
        if ix < 0.0 || iy < 0.0 || ix >= w as f32 || iy >= h as f32 {
            0.0
        } else {
            buf[iy as usize * w + ix as usize] as f32
        }
    };
    let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
    let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
    (top * (1.0 - fy) + bottom * fy) / 255.0
}

/// The mockup's `fitPx(str, max, width = 150)`: the largest canvas px (≤ max)
/// that roughly fits `str` in `width` canvas units.
pub fn fit_px(text: &str, max: u16, width: f32) -> u16 {
    let len = text.chars().count().max(1) as f32;
    (floorf(width / (len * 0.58)) as u16).min(max)
}

#[allow(dead_code)]
const _: () = assert!(PANEL_WIDTH == 96);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_px_matches_the_mockup() {
        // fitPx("d20", 50) → min(50, floor(150 / 1.74)) = 50
        assert_eq!(fit_px("d20", 50, 150.0), 50);
        // fitPx("Pass the Pot ×2", 50) → floor(150 / (15 × 0.58)) = 17
        assert_eq!(fit_px("Pass the Pot ×2", 50, 150.0), 17);
        // fitPx("SUGARCUBE", 28, 142) → floor(142 / 5.22) = 27
        assert_eq!(fit_px("SUGARCUBE", 28, 142.0), 27);
    }
}
