//! The bits of the CGB picture processor the world renderer needs: reading
//! tile rows out of VRAM and drawing objects over a background with the
//! CGB's priority rules, the same way walnut-cgb's scanline renderer does.

use crate::mem::GbMem;
use crate::view::{View, VOID};

/// BG/window tile attribute bits (and OAM's, which share most of them).
pub mod attr {
    pub const PALETTE: u8 = 0x07;
    pub const BANK1: u8 = 0x08;
    /// OAM only: DMG palette OBP1 (ignored in CGB mode).
    pub const OBP1: u8 = 0x10;
    pub const XFLIP: u8 = 0x20;
    pub const YFLIP: u8 = 0x40;
    /// BG: drawn over objects. OAM: drawn behind BG colours 1–3.
    pub const PRIORITY: u8 = 0x80;
}

/// Canvas pixels store a CGB palette index (0–31 background, 32–63 objects)
/// plus this flag on background pixels whose tile has the priority bit.
pub const BG_PRIORITY: u8 = 0x40;
pub const OBJ: u8 = 0x20;

/// VRAM offset (from `$8000`) of background tile `tile`, given LCDC.
#[inline]
pub fn bg_tile_offset(lcdc: u8, tile: u8, attr: u8) -> usize {
    let base = if lcdc & 0x10 != 0 {
        tile as usize * 16
    } else {
        0x1000usize.wrapping_add((tile as i8 as isize * 16) as usize)
    };
    base + if attr & attr::BANK1 != 0 { 0x2000 } else { 0 }
}

/// The eight colour numbers (0–3) of row `row` of the tile at VRAM offset
/// `off`, left to right, honouring the flips in `attr`.
#[inline]
pub fn tile_row(vram: &[u8], off: usize, row: usize, attr: u8, height: usize) -> [u8; 8] {
    let r = if attr & attr::YFLIP != 0 {
        height - 1 - row
    } else {
        row
    };
    let lo = vram[off + 2 * r];
    let hi = vram[off + 2 * r + 1];
    let mut out = [0u8; 8];
    for (i, px) in out.iter_mut().enumerate() {
        let bit = if attr & attr::XFLIP != 0 { i } else { 7 - i };
        *px = ((lo >> bit) & 1) | (((hi >> bit) & 1) << 1);
    }
    out
}

/// Byte `b`'s bits spread one per byte, leftmost pixel (bit 7) in the low
/// byte: `SPREAD[b] | SPREAD[c] << 1` is a tile row's eight colour numbers
/// as eight bytes.
const SPREAD: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut b = 0;
    while b < 256 {
        let mut i = 0;
        while i < 8 {
            t[b] |= (((b >> (7 - i)) & 1) as u64) << (8 * i);
            i += 1;
        }
        b += 1;
    }
    t
};

/// [`tile_row`] eight pixels at a time: the row's colour numbers as the
/// bytes of a `u64`, leftmost pixel in the low byte.
#[inline]
pub fn tile_row_packed(vram: &[u8], off: usize, row: usize, attr: u8, height: usize) -> u64 {
    let r = if attr & attr::YFLIP != 0 {
        height - 1 - row
    } else {
        row
    };
    let px = SPREAD[vram[off + 2 * r] as usize] | SPREAD[vram[off + 2 * r + 1] as usize] << 1;
    if attr & attr::XFLIP != 0 {
        px.swap_bytes()
    } else {
        px
    }
}

/// One object as OAM holds it: `x`/`y` are the screen position of its
/// top-left pixel (OAM's x − 8, y − 16), but wide enough to sit off screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Obj {
    pub x: i32,
    pub y: i32,
    pub tile: u8,
    pub attr: u8,
}

/// Draw `objs` (OAM order: earlier entries on top) onto `view`, where the
/// screen's top-left pixel is at canvas `origin`. Objects over [`VOID`]
/// (fog) aren't drawn. Unlike the real PPU there's no ten-per-line limit.
pub fn draw_objs<M: GbMem + ?Sized>(m: &M, view: &mut View, objs: &[Obj], origin: (i32, i32)) {
    let lcdc = m.io()[0x40];
    if lcdc & 0x02 == 0 {
        return;
    }
    let tall = lcdc & 0x04 != 0;
    // CGB: with LCDC bit 0 clear, objects always win over the background.
    let bg_master = lcdc & 0x01 != 0;
    let height = if tall { 16 } else { 8 };
    let vram = m.vram();
    // The PPU draws from the lowest priority up, so the first entry ends on
    // top.
    for o in objs.iter().rev() {
        let tile = if tall { o.tile & 0xFE } else { o.tile };
        let off = tile as usize * 16 + if o.attr & attr::BANK1 != 0 { 0x2000 } else { 0 };
        let pal = OBJ | ((o.attr & attr::PALETTE) << 2);
        for row in 0..height {
            let cy = origin.1 + o.y + row as i32;
            if !(0..crate::view::V as i32).contains(&cy) {
                continue;
            }
            // An 8×16 object's second tile follows the first in VRAM; the
            // y flip swaps them, which `tile_row` does over 16 rows.
            let px = tile_row(vram, off, row, o.attr, height);
            for (i, &c) in px.iter().enumerate() {
                let cx = origin.0 + o.x + i as i32;
                if c == 0 || !(0..crate::view::V as i32).contains(&cx) {
                    continue;
                }
                let p = &mut view.idx[cy as usize * crate::view::V + cx as usize];
                if *p == VOID {
                    continue;
                }
                let under = *p & 0x03;
                let bg_prio = *p & BG_PRIORITY != 0;
                if bg_master && under != 0 && (bg_prio || o.attr & attr::PRIORITY != 0) {
                    continue;
                }
                *p = (*p & BG_PRIORITY) | pal | c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_rows_decode_2bpp_and_flip() {
        // Row 0: lo = 0b1000_0001, hi = 0b1100_0000 → 3 1 0 0 0 0 0 1
        let mut vram = [0u8; 32];
        vram[0] = 0b1000_0001;
        vram[1] = 0b1100_0000;
        assert_eq!(tile_row(&vram, 0, 0, 0, 8), [3, 2, 0, 0, 0, 0, 0, 1]);
        assert_eq!(tile_row(&vram, 0, 0, attr::XFLIP, 8), [1, 0, 0, 0, 0, 0, 2, 3]);
        assert_eq!(tile_row(&vram, 0, 7, attr::YFLIP, 8), [3, 2, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn packed_rows_match_the_plain_decoder() {
        let mut vram = [0u8; 32];
        for (i, b) in vram.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(0x5B) ^ 0xA6;
        }
        for a in [0, attr::XFLIP, attr::YFLIP, attr::XFLIP | attr::YFLIP] {
            for row in 0..8 {
                let packed = tile_row_packed(&vram, 0, row, a, 8).to_le_bytes();
                assert_eq!(packed, tile_row(&vram, 0, row, a, 8));
            }
        }
    }

    #[test]
    fn signed_tile_addressing() {
        assert_eq!(bg_tile_offset(0x00, 0x00, 0), 0x1000);
        assert_eq!(bg_tile_offset(0x00, 0x80, 0), 0x0800);
        assert_eq!(bg_tile_offset(0x10, 0x80, 0), 0x0800);
        assert_eq!(bg_tile_offset(0x10, 0x01, attr::BANK1), 0x2010);
    }
}
