//! Drawing the cube's own cards onto faces: text in the game's font (read
//! from VRAM, 1:1), Pokémon pictures, icons, bars.

use crate::crystal::mons::{Pic, Text};
use crate::geom::{FaceXf, Layout, Role};
use crate::mem::{reg, GbMem};
use crate::ppu::{bg_tile_offset, tile_row};
use crate::{FaceBuf, FACE};

/// One face, upright, shifted `dx` pixels right (for a card sliding on or
/// off). Everything is clipped to the face.
pub struct Surface<'a> {
    buf: &'a mut FaceBuf,
    xf: FaceXf,
    dx: i32,
}

impl<'a> Surface<'a> {
    pub fn new(faces: &'a mut [FaceBuf; 6], layout: &Layout, role: Role, dx: i32) -> Surface<'a> {
        let i = layout.face_with(role).index();
        Surface {
            buf: &mut faces[i],
            xf: layout.xf[i],
            dx,
        }
    }

    #[inline]
    pub fn put(&mut self, u: i32, v: i32, c: u16) {
        let u = u + self.dx;
        if (0..FACE as i32).contains(&u) && (0..FACE as i32).contains(&v) {
            self.buf[self.xf.index(u as usize, v as usize)] = c;
        }
    }

    pub fn rect(&mut self, u: i32, v: i32, w: i32, h: i32, c: u16) {
        for y in v..v + h {
            for x in u..u + w {
                self.put(x, y, c);
            }
        }
    }

    /// The whole card (the face's 64×64 at this shift).
    pub fn fill(&mut self, c: u16) {
        self.rect(0, 0, FACE as i32, FACE as i32, c);
    }

    /// A 56×56 picture with its top-left at (`u`, `v`). Colour 0 is drawn
    /// too (pictures sit on white).
    pub fn pic(&mut self, pic: &Pic, pal: &[u16; 4], u: i32, v: i32) {
        for y in 0..Pic::SIDE {
            for x in 0..Pic::SIDE {
                self.put(u + x as i32, v + y as i32, pal[pic.pixel(x, y) as usize]);
            }
        }
    }

    /// A 16×16 icon scaled `k` times: rows of `' '` (nothing) and `'1'`–
    /// `'3'` (`pal[0..3]`).
    pub fn icon(&mut self, rows: &[&str; 16], pal: &[u16; 3], u: i32, v: i32, k: i32) {
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.bytes().enumerate() {
                if let b'1'..=b'3' = ch {
                    self.rect(
                        u + x as i32 * k,
                        v + y as i32 * k,
                        k,
                        k,
                        pal[(ch - b'1') as usize],
                    );
                }
            }
        }
    }

    /// A bar `w` wide: an outline, filled `fill`/`max`. `c` is the fill,
    /// the outline and the empty part's colours.
    pub fn bar(&mut self, u: i32, v: i32, w: i32, (fill, max): (u16, u16), c: [u16; 3]) {
        self.rect(u, v, w, 5, c[1]);
        self.rect(u + 1, v + 1, w - 2, 3, c[2]);
        let n = if max == 0 {
            0
        } else {
            ((w - 2) as u32 * fill.min(max) as u32).div_ceil(max as u32) as i32
        };
        self.rect(u + 1, v + 1, n, 3, c[0]);
    }
}

/// The game's font as it sits in VRAM, in one BG palette.
pub struct Font<'a, M: GbMem + ?Sized> {
    m: &'a M,
    lcdc: u8,
    pub pal: [u16; 4],
}

impl<'a, M: GbMem + ?Sized> Font<'a, M> {
    /// The font in BG palette `attr & 7`.
    pub fn new(m: &'a M, attr: u8) -> Self {
        let p = m.bg_palette();
        let i = (attr & 7) as usize * 8;
        let c =
            |k: usize| crate::color::bgr555_to_rgb565(u16::from_le_bytes([p[i + 2 * k], p[i + 2 * k + 1]]));
        Font {
            m,
            lcdc: m.reg(reg::LCDC),
            pal: [c(0), c(1), c(2), c(3)],
        }
    }

    pub fn paper(&self) -> u16 {
        self.pal[0]
    }

    fn row(&self, code: u8, r: usize) -> [u8; 8] {
        // Font tiles are in VRAM bank 0, attributes don't apply.
        tile_row(self.m.vram(), bg_tile_offset(self.lcdc, code, 0), r, 0, 8)
    }

    /// Columns `from..8` of `code` are all it needs.
    fn fits(&self, code: u8, from: usize, to: usize) -> bool {
        (0..8).all(|r| {
            let px = self.row(code, r);
            px[..from].iter().chain(&px[to..]).all(|&c| c == 0)
        })
    }

    /// Glyph columns `from..to` at (`u`, `v`).
    #[allow(clippy::too_many_arguments)]
    fn glyph(&self, s: &mut Surface, code: u8, u: i32, v: i32, from: usize, to: usize, invert: bool) {
        for r in 0..8 {
            let px = self.row(code, r);
            for (i, &c) in px[from..to].iter().enumerate() {
                s.put(
                    u + i as i32,
                    v + r as i32,
                    self.pal[if invert { 3 - c } else { c } as usize],
                );
            }
        }
    }

    /// The columns of each glyph to draw so `codes` fits `width`: all 8 if
    /// there's room, else the font's blank right-hand column dropped, then
    /// its blank left one. Text is never scaled; with too little room even
    /// so, the end is cut.
    fn columns(&self, codes: &[u8], width: i32) -> (usize, usize) {
        for (from, to) in [(0, 8), (0, 7), (1, 7)] {
            let fits = codes.len() as i32 * (to - from) as i32 <= width;
            if fits && codes.iter().all(|&c| self.fits(c, from, to)) {
                return (from, to);
            }
        }
        (0, 8)
    }

    /// `codes` from (`u`, `v`), packed to fit `width`; returns its width.
    pub fn text(&self, s: &mut Surface, codes: &[u8], u: i32, v: i32, width: i32, invert: bool) -> i32 {
        let (from, to) = self.columns(codes, width);
        let pitch = (to - from) as i32;
        let n = (codes.len() as i32).min(width / pitch);
        for (k, &c) in codes.iter().take(n as usize).enumerate() {
            self.glyph(s, c, u + k as i32 * pitch, v, from, to, invert);
        }
        n * pitch
    }

    /// `codes` centred on the face at row `v`.
    pub fn centred(&self, s: &mut Surface, codes: &[u8], v: i32, invert: bool) {
        let (from, to) = self.columns(codes, FACE as i32);
        let w = (codes.len() * (to - from)).min(FACE) as i32;
        self.text(s, codes, (FACE as i32 - w) / 2, v, FACE as i32, invert);
    }

    pub fn centred_text(&self, s: &mut Surface, t: &Text, v: i32) {
        self.centred(s, t.as_slice(), v, false);
    }

    /// A glyph `k` times its size, centred on (`cu`, `cv`) (the "?" of an
    /// unknown Pokémon: an icon, not text).
    pub fn big(&self, s: &mut Surface, code: u8, cu: i32, cv: i32, k: i32) {
        for r in 0..8 {
            let px = self.row(code, r);
            for (i, &c) in px.iter().enumerate() {
                if c != 0 {
                    s.rect(
                        cu - 4 * k + i as i32 * k,
                        cv - 4 * k + r as i32 * k,
                        k,
                        k,
                        self.pal[c as usize],
                    );
                }
            }
        }
    }
}
