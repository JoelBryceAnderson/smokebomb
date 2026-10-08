//! The canvas the faces sample: 192×192 Game Boy pixels centred on the
//! middle of the up face, so it holds the up face's 64×64 plus 64 px past
//! each of its edges (what the side faces show).
//!
//! Pixels are CGB palette indexes (see [`crate::ppu`]), [`VOID`] where
//! there's nothing to show. A coarse light grid fades the picture to black
//! as it nears the void, so the edge of the known world is a soft fog
//! rather than a hard cut. On the die this canvas is 36 KiB.

use crate::color::dim;
use crate::geom::{drape, Layout, Role};
use crate::{FaceBuf, FACE, FACE_PIXELS, LCD_H, LCD_W};

/// Canvas side, pixels.
pub const V: usize = 192;
/// Canvas position of the up face's centre.
pub const MID: i32 = (V / 2) as i32;
/// Light grid spacing.
pub const CELL: usize = 8;
pub const CELLS: usize = V / CELL;
const CORNERS: usize = CELLS + 1;

/// Nothing here: drawn black.
pub const VOID: u8 = 0xFF;

#[derive(Clone)]
pub struct View {
    pub idx: [u8; V * V],
    /// The 64 CGB colours the indexes refer to, RGB565.
    pub palette: [u16; 64],
    /// Brightness at each light-grid corner, 0–255.
    light: [u8; CORNERS * CORNERS],
    /// Cells whose four corners are all fully lit: no fog maths there.
    clear: [bool; CELLS * CELLS],
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

impl View {
    pub const fn new() -> View {
        View {
            idx: [VOID; V * V],
            palette: [0; 64],
            light: [255; CORNERS * CORNERS],
            clear: [true; CELLS * CELLS],
        }
    }

    pub fn clear(&mut self) {
        self.idx.fill(VOID);
    }

    /// Fill from a 160×144 frame of palette indexes with frame pixel
    /// `centre` at the canvas middle. Everything off the frame is void.
    pub fn from_frame(&mut self, frame: &[u8; LCD_W * LCD_H], palette: &[u16; 64], centre: (i32, i32)) {
        self.palette = *palette;
        let ox = centre.0 - MID;
        let oy = centre.1 - MID;
        for y in 0..V {
            let fy = oy + y as i32;
            let row = &mut self.idx[y * V..(y + 1) * V];
            if !(0..LCD_H as i32).contains(&fy) {
                row.fill(VOID);
                continue;
            }
            for (x, p) in row.iter_mut().enumerate() {
                let fx = ox + x as i32;
                *p = if (0..LCD_W as i32).contains(&fx) {
                    frame[fy as usize * LCD_W + fx as usize] & 0x3F
                } else {
                    VOID
                };
            }
        }
    }

    /// Recompute the fog from where the void is. Within `fog_px` of a void
    /// cell the picture darkens to black at the void's edge.
    pub fn compute_light(&mut self, fog_px: u8) {
        let mut open = [false; CELLS * CELLS];
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                let p = self.idx[(cy * CELL + CELL / 2) * V + cx * CELL + CELL / 2];
                open[cy * CELLS + cx] = p != VOID;
            }
        }
        // Chebyshev distance, in cells, from each corner to the nearest void
        // cell (a corner touching a void cell is 0). Off the canvas counts
        // as open: the far edge of a side face isn't the world's edge.
        let reach = (fog_px as usize).div_ceil(CELL).max(1);
        let mut dist = [u8::MAX; CORNERS * CORNERS];
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                if open[cy * CELLS + cx] {
                    continue;
                }
                let x0 = cx.saturating_sub(reach);
                let y0 = cy.saturating_sub(reach);
                for ky in y0..=(cy + 1 + reach).min(CELLS) {
                    for kx in x0..=(cx + 1 + reach).min(CELLS) {
                        let dx = if kx < cx {
                            cx - kx
                        } else {
                            kx.saturating_sub(cx + 1)
                        };
                        let dy = if ky < cy {
                            cy - ky
                        } else {
                            ky.saturating_sub(cy + 1)
                        };
                        let d = dx.max(dy) as u8;
                        let slot = &mut dist[ky * CORNERS + kx];
                        *slot = (*slot).min(d);
                    }
                }
            }
        }
        for (l, &d) in self.light.iter_mut().zip(dist.iter()) {
            *l = if d == u8::MAX || fog_px == 0 {
                255
            } else {
                let lit = (d as u32 * CELL as u32 * 255 / fog_px as u32).min(255);
                // Ease in, so the fade looks like fog rather than a ramp.
                ((lit * lit) / 255) as u8
            };
        }
        for cy in 0..CELLS {
            for cx in 0..CELLS {
                let l = |i: usize, j: usize| self.light[j * CORNERS + i];
                self.clear[cy * CELLS + cx] = l(cx, cy) == 255
                    && l(cx + 1, cy) == 255
                    && l(cx, cy + 1) == 255
                    && l(cx + 1, cy + 1) == 255;
            }
        }
    }

    /// Brightness at canvas pixel (`x`, `y`): the light grid, bilinear.
    #[inline]
    pub fn light_at(&self, x: usize, y: usize) -> u8 {
        let (cx, cy) = (x / CELL, y / CELL);
        let (fx, fy) = ((2 * (x % CELL) + 1) as u32, (2 * (y % CELL) + 1) as u32);
        let n = 2 * CELL as u32;
        let l = |i: usize, j: usize| self.light[j * CORNERS + i] as u32;
        let top = l(cx, cy) * (n - fx) + l(cx + 1, cy) * fx;
        let bot = l(cx, cy + 1) * (n - fx) + l(cx + 1, cy + 1) * fx;
        ((top * (n - fy) + bot * fy) / (n * n)) as u8
    }

    /// The colour of canvas pixel (`x`, `y`) with fog.
    #[inline]
    pub fn shade(&self, x: usize, y: usize) -> u16 {
        let p = self.idx[y * V + x];
        if p == VOID {
            return 0;
        }
        dim(self.palette[(p & 0x3F) as usize], self.light_at(x, y))
    }

    /// The colour without fog (void is black).
    #[inline]
    pub fn raw(&self, x: usize, y: usize) -> u16 {
        let p = self.idx[y * V + x];
        if p == VOID {
            0
        } else {
            self.palette[(p & 0x3F) as usize]
        }
    }

    /// Drape the canvas over the faces: the up face and the four sides.
    /// Faces whose bit is set in `skip` are left alone, as is the bottom
    /// face.
    ///
    /// Each face is an affine walk over the canvas (the drape is a quarter
    /// turn and a shift per face), and the fog maths only runs in cells the
    /// fog reaches: this is the per-frame hot loop on the die.
    pub fn drape_onto(&self, layout: &Layout, faces: &mut [FaceBuf; 6], skip: u8) {
        for (i, face) in faces.iter_mut().enumerate() {
            let role = layout.role[i];
            if skip & (1 << i) != 0 || role == Role::Bottom {
                continue;
            }
            let xf = layout.xf[i];
            let at = |u: i32, v: i32| {
                let (dx, dy) = drape(role, u, v).expect("not the bottom");
                (MID + dx, MID + dy)
            };
            let (x0, y0) = at(0, 0);
            let (xu, yu) = at(1, 0);
            let (xv, yv) = at(0, 1);
            let (du, dv) = ((xu - x0, yu - y0), (xv - x0, yv - y0));
            let last = FACE as i32 - 1;
            for v in 0..FACE as i32 {
                let (mut x, mut y) = (x0 + v * dv.0, y0 + v * dv.1);
                let mut out = xf.base + xf.dv * v;
                // Both ends of the row are on the canvas and in the face, so
                // everything between is: index without bounds checks.
                let (xe, ye, oe) = (x + last * du.0, y + last * du.1, out + last * xf.du);
                assert!(
                    [x, xe].iter().all(|c| (0..V as i32).contains(c))
                        && [y, ye].iter().all(|c| (0..V as i32).contains(c))
                        && [out, oe].iter().all(|o| (0..FACE_PIXELS as i32).contains(o))
                );
                for _ in 0..FACE {
                    let (cx, cy) = (x as usize, y as usize);
                    // SAFETY: (cx, cy) and `out` lie between the row's two
                    // ends, both checked above; `p & 0x3F` < 64; the cell
                    // index of an on-canvas pixel is < CELLS².
                    unsafe {
                        let p = *self.idx.get_unchecked(cy * V + cx);
                        *face.get_unchecked_mut(out as usize) = if p == VOID {
                            0
                        } else if *self.clear.get_unchecked((cy / CELL) * CELLS + cx / CELL) {
                            *self.palette.get_unchecked((p & 0x3F) as usize)
                        } else {
                            dim(
                                *self.palette.get_unchecked((p & 0x3F) as usize),
                                self.light_at(cx, cy),
                            )
                        };
                    }
                    x += du.0;
                    y += du.1;
                    out += xf.du;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_centre_lands_mid_canvas_and_off_frame_is_void() {
        let mut f = [0u8; LCD_W * LCD_H];
        f[72 * LCD_W + 72] = 5;
        let mut v = View::new();
        v.from_frame(&f, &[0; 64], (72, 72));
        assert_eq!(v.idx[MID as usize * V + MID as usize], 5);
        assert_eq!(v.idx[0], VOID); // (-24, -24)
        assert_eq!(v.idx[24 * V + 24], 0); // frame (0, 0)
    }

    #[test]
    fn the_fast_drape_matches_pixel_by_pixel_shading() {
        use crate::geom::{Heading, Layout};
        let mut f = [0u8; LCD_W * LCD_H];
        for (i, p) in f.iter_mut().enumerate() {
            *p = ((i * 7 + i / LCD_W) % 64) as u8;
        }
        let mut pal = [0u16; 64];
        for (i, c) in pal.iter_mut().enumerate() {
            *c = (i as u16).wrapping_mul(0x9E37);
        }
        let mut v = View::new();
        v.from_frame(&f, &pal, (60, 50));
        v.compute_light(16);
        for h in [Heading::default(), Heading::default().rolled_to([1, 0, 0])] {
            let layout = Layout::new(h);
            let mut fast = [[0u16; FACE * FACE]; 6];
            v.drape_onto(&layout, &mut fast, 0);
            for (i, face) in fast.iter().enumerate() {
                let role = layout.role[i];
                if role == Role::Bottom {
                    continue;
                }
                for vv in 0..FACE {
                    for u in 0..FACE {
                        let (dx, dy) = drape(role, u as i32, vv as i32).unwrap();
                        let want = v.shade((MID + dx) as usize, (MID + dy) as usize);
                        assert_eq!(face[layout.xf[i].index(u, vv)], want);
                    }
                }
            }
        }
    }

    #[test]
    fn fog_fades_to_black_at_the_void_and_is_clear_far_away() {
        let mut f = [1u8; LCD_W * LCD_H];
        f[0] = 1;
        let mut v = View::new();
        let pal = [0xFFFF; 64];
        v.from_frame(&f, &pal, (72, 72));
        v.compute_light(16);
        // Middle of the frame: fully lit.
        assert_eq!(v.light_at(96, 96), 255);
        // The frame's top-left pixel (canvas 24, 24) sits on the void edge.
        assert!(v.light_at(24, 24) < 40, "{}", v.light_at(24, 24));
        // Brightening inward.
        assert!(v.light_at(30, 30) > v.light_at(25, 25));
        assert_eq!(v.shade(0, 0), 0);
    }
}
