//! Crystal's full-screen scenes, laid out for the cube (part of C,
//! [`UiStyle::Front`](crate::fallback::UiStyle::Front)).
//!
//! * **The naming screen** (your name, your rival's, a nickname, a box):
//!   the prompt and the name so far on the up face, the keyboard on the
//!   front face, packed so all nine columns fit 64 px, the key under the
//!   cursor drawn inverted. Read from `wTilemap`, the cursor from its
//!   sprite animation struct (it's a sprite, not a tile).
//! * **Stills**: the new-game speech, the main menu, and any other screen
//!   with a framed box. The frame is folded round the cube with the up face
//!   on the picture (Oak, the Pokémon, you; their 7×7-tile pictures fit a
//!   face at 1:1) and the active box re-flowed onto the front face as in C.
//!   Screens without a box (the opening movie, the title screen) get the
//!   frame folded round its centre, fixed: no panning after whatever moved.
//!
//! "Front" is the south face, as in [`crate::fallback`].

use crate::crystal::charmap;
use crate::crystal::screen::{attrmap, tilemap, COLS, ROWS};
use crate::crystal::syms;
use crate::fallback::{self, find_cursor, pick_region, Fallback, LINE};
use crate::geom::{Compass, FaceXf, Layout, Role};
use crate::mem::{reg, GbMem, Sym};
use crate::ppu::{bg_tile_offset, tile_row};
use crate::view::{View, MID, V};
use crate::{FaceBuf, FACE, LCD_H, LCD_W};

/// The naming screen's border tile (`NAMINGSCREEN_BORDER`, ■).
const BORDER: u8 = 0x60;
/// Keyboard: letters every other column from here, every other row.
const KB_COL: usize = 2;
const KEYS: usize = 9;
/// The UPPER/lower, DEL, END row.
const CMD_ROW: usize = 16;
/// Sprite animation struct fields (`sprite_anim_struct` in macros/ram.asm).
const ANIM_VAR1: u16 = 12;
const ANIM_VAR2: u16 = 13;
const ANIM_LEN: u16 = 16;

/// Draws background tiles from `wTilemap` onto faces at 1:1.
struct Painter<'a, M: GbMem + ?Sized> {
    m: &'a M,
    lcdc: u8,
}

impl<'a, M: GbMem + ?Sized> Painter<'a, M> {
    fn new(m: &'a M) -> Self {
        Painter {
            m,
            lcdc: m.reg(reg::LCDC),
        }
    }

    fn colour(&self, pal: u8, c: u8) -> u16 {
        let i = ((pal & 7) as usize * 4 + c as usize) * 2;
        let p = self.m.bg_palette();
        crate::color::bgr555_to_rgb565(u16::from_le_bytes([p[i], p[i + 1]]))
    }

    fn row(&self, x: usize, y: usize, r: usize) -> ([u8; 8], u8) {
        let a = attrmap(self.m, x, y);
        let off = bg_tile_offset(self.lcdc, tilemap(self.m, x, y), a);
        (tile_row(self.m.vram(), off, r, a, 8), a)
    }

    /// The tile at `wTilemap` (`x`, `y`) with its right-hand column empty
    /// (the font's letter spacing), so it can sit 7 px from the next.
    fn narrow(&self, x: usize, y: usize) -> bool {
        (0..8).all(|r| self.row(x, y, r).0[7] == 0)
    }

    /// Draw the leftmost `width` columns of the tile at `wTilemap` (`x`,
    /// `y`) with its top-left at upright (`u0`, `v0`), colours inverted if
    /// `invert` (the highlighted key).
    #[allow(clippy::too_many_arguments)]
    fn tile(
        &self,
        face: &mut FaceBuf,
        xf: FaceXf,
        u0: usize,
        v0: usize,
        x: usize,
        y: usize,
        width: usize,
        invert: bool,
    ) {
        for r in 0..8 {
            let (px, a) = self.row(x, y, r);
            for (i, &c) in px.iter().enumerate().take(width) {
                let (u, v) = (u0 + i, v0 + r);
                if u < FACE && v < FACE {
                    face[xf.index(u, v)] = self.colour(a, if invert { 3 - c } else { c });
                }
            }
        }
    }
}

/// The naming screen, as `NamingScreen_InitText` lays it out: ■ all round,
/// the prompt and the name in the top box, the keyboard below (a box name
/// has a sixth row and moves up two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Naming {
    /// Tile row of the first row of keys; the others follow every 2 rows.
    kb: usize,
    /// Rows of letters: 4, or 5 for a box name. Cursor row `rows` is the
    /// command row.
    rows: usize,
    /// (column 0–8, row), if the cursor's struct reads sensibly.
    cursor: Option<(usize, usize)>,
    /// Where the name is printed, its length in slots, characters typed.
    entry: (usize, usize),
    slots: usize,
    typed: usize,
}

impl Naming {
    /// The naming screen, if that's what `wTilemap` holds.
    pub fn read<M: GbMem + ?Sized>(m: &M) -> Option<Naming> {
        let framed = (0..COLS).all(|x| tilemap(m, x, 0) == BORDER && tilemap(m, x, ROWS - 1) == BORDER)
            && (0..ROWS).all(|y| tilemap(m, 0, y) == BORDER && tilemap(m, COLS - 1, y) == BORDER);
        if !framed {
            return None;
        }
        // "A B C…" or "a b c…" ('A' is $80, 'a' $A0).
        let letters = |y: usize| {
            matches!(tilemap(m, KB_COL, y), 0x80 | 0xA0) && matches!(tilemap(m, KB_COL + 2, y), 0x81 | 0xA1)
        };
        let (kb, rows) = if letters(6) {
            (6, 5)
        } else if letters(8) {
            (8, 4)
        } else {
            return None;
        };
        // The rest is in a union of WRAM that other screens reuse: only
        // believed where it's in range.
        let p = m.word(syms::W_NAMING_SCREEN_CURSOR_OBJECT_POINTER);
        let base = syms::W_SPRITE_ANIMATION_STRUCTS.addr;
        let cursor = (p >= base && p < syms::W_SPRITE_ANIM_DATA_END.addr && (p - base) % ANIM_LEN == 0)
            .then(|| {
                let s = Sym::new(0, p);
                (
                    m.byte(s.offset(ANIM_VAR1)) as usize,
                    m.byte(s.offset(ANIM_VAR2)) as usize,
                )
            })
            .filter(|&(c, r)| c < KEYS && r <= rows);
        let at = m
            .word(syms::W_NAMING_SCREEN_STRING_ENTRY_COORD)
            .wrapping_sub(syms::W_TILEMAP.addr) as usize;
        let entry = if at < COLS * ROWS && at / COLS < kb {
            (at % COLS, at / COLS)
        } else {
            (5, kb - 2)
        };
        let slots = match m.byte(syms::W_NAMING_SCREEN_MAX_NAME_LENGTH) as usize {
            n @ 1..=10 => n,
            _ => 7,
        }
        .min(COLS - 1 - entry.0);
        let typed = (m.byte(syms::W_NAMING_SCREEN_CUR_NAME_LENGTH) as usize).min(slots);
        Some(Naming {
            kb,
            rows,
            cursor,
            entry,
            slots,
            typed,
        })
    }

    pub fn draw<M: GbMem + ?Sized>(&self, m: &M, layout: &Layout, faces: &mut [FaceBuf; 6]) {
        let p = Painter::new(m);
        let paper = p.colour(attrmap(m, KB_COL, self.kb), 0);
        for (i, f) in faces.iter_mut().enumerate() {
            if layout.role[i] != Role::Bottom {
                f.fill(paper);
            }
        }

        // Up face: the prompt ("YOUR NAME?"), words wrapped to 8, centred;
        // then the name, scrolled to keep the next slot in view.
        let top = layout.face_with(Role::Top).index();
        let xf = layout.xf[top];
        let mut words = Words::new();
        for y in 1..self.entry.1 {
            for x in 1..COLS - 1 {
                words.feed(tilemap(m, x, y), x, y);
            }
            words.feed(charmap::SPACE, 0, 0);
        }
        let shown = words.n.min(3);
        for (li, line) in words.lines[words.n - shown..words.n].iter().enumerate() {
            let u0 = (FACE - line.len * 8) / 2;
            for (ci, &(x, y)) in line.at[..line.len].iter().enumerate() {
                if (x, y) == SPACE_AT {
                    continue;
                }
                p.tile(&mut faces[top], xf, u0 + ci * 8, 4 + li * 9, x, y, 8, false);
            }
        }
        let w = self.slots.min(LINE);
        let first = (self.typed + 1).saturating_sub(w).min(self.slots - w);
        let u0 = (FACE - w * 8) / 2;
        for i in 0..w {
            let (x, y) = (self.entry.0 + first + i, self.entry.1);
            p.tile(&mut faces[top], xf, u0 + i * 8, 44, x, y, 8, false);
        }

        // Front face: the keyboard. Nine keys at a 7 px pitch fit if the
        // font leaves its usual blank column; otherwise eight at 8 px,
        // scrolled to keep the cursor's column.
        let front = layout.face_with(Role::Side(Compass::South)).index();
        let xf = layout.xf[front];
        let key = |c: usize, r: usize| (KB_COL + 2 * c, self.kb + 2 * r);
        let narrow = (0..self.rows).all(|r| {
            (0..KEYS).all(|c| {
                let (x, y) = key(c, r);
                p.narrow(x, y)
            })
        });
        let (pitch, n, first) = if narrow {
            (7, KEYS, 0)
        } else {
            (
                8,
                KEYS - 1,
                usize::from(self.cursor.is_some_and(|(c, _)| c == KEYS - 1)),
            )
        };
        let u0 = (FACE - n * pitch) / 2;
        for r in 0..self.rows {
            for c in first..first + n {
                let (x, y) = key(c, r);
                let on = self.cursor == Some((c, r));
                p.tile(
                    &mut faces[front],
                    xf,
                    u0 + (c - first) * pitch,
                    r * 8,
                    x,
                    y,
                    pitch,
                    on,
                );
            }
        }
        // UPPER/lower, DEL, END: a line each under the letters, the one
        // under the cursor (columns 0–2, 3–5, 6–8) inverted.
        let mut runs = [(0usize, 0usize); 3];
        let mut nr = 0;
        let mut x = KB_COL;
        while x < COLS - 1 && nr < runs.len() {
            if tilemap(m, x, CMD_ROW) == charmap::SPACE {
                x += 1;
                continue;
            }
            let start = x;
            while x < COLS - 1 && tilemap(m, x, CMD_ROW) != charmap::SPACE {
                x += 1;
            }
            runs[nr] = (start, (x - start).min(LINE));
            nr += 1;
        }
        let gap = usize::from(self.rows + nr < LINE);
        for (k, &(x0, len)) in runs[..nr].iter().enumerate() {
            let on = self.cursor.is_some_and(|(c, r)| r == self.rows && c / 3 == k);
            let u0 = (FACE - len * 8) / 2;
            for i in 0..len {
                p.tile(
                    &mut faces[front],
                    xf,
                    u0 + i * 8,
                    (self.rows + gap + k) * 8,
                    x0 + i,
                    CMD_ROW,
                    8,
                    on,
                );
            }
        }
    }
}

/// A space put between words: nothing drawn.
const SPACE_AT: (usize, usize) = (usize::MAX, 0);

/// Text wrapped to lines of 8 at word breaks (a longer word is split).
struct Words {
    lines: [WordLine; 6],
    n: usize,
    word: WordLine,
}

#[derive(Clone, Copy)]
struct WordLine {
    at: [(usize, usize); LINE],
    len: usize,
}

impl WordLine {
    const EMPTY: WordLine = WordLine {
        at: [(0, 0); LINE],
        len: 0,
    };
}

impl Words {
    fn new() -> Self {
        Words {
            lines: [WordLine::EMPTY; 6],
            n: 0,
            word: WordLine::EMPTY,
        }
    }

    /// The next tile in reading order; a space ends a word.
    fn feed(&mut self, tile: u8, x: usize, y: usize) {
        if tile != charmap::SPACE && tile != BORDER {
            if self.word.len == LINE {
                self.flush();
            }
            self.word.at[self.word.len] = (x, y);
            self.word.len += 1;
        } else {
            self.flush();
        }
    }

    fn flush(&mut self) {
        let w = core::mem::replace(&mut self.word, WordLine::EMPTY);
        if w.len == 0 {
            return;
        }
        if self.n > 0 {
            let last = &mut self.lines[self.n - 1];
            if last.len + 1 + w.len <= LINE {
                last.len += 1;
                last.at[last.len - 1] = SPACE_AT;
                last.at[last.len..last.len + w.len].copy_from_slice(&w.at[..w.len]);
                last.len += w.len;
                return;
            }
        }
        if self.n == self.lines.len() {
            self.lines.copy_within(1.., 0);
            self.n -= 1;
        }
        self.lines[self.n] = w;
        self.n += 1;
    }
}

/// A still screen (see the module docs). Returns false, drawing nothing,
/// for a screen with a ▶ cursor and no box (the party menu and the like),
/// which A's panning after the cursor shows better.
#[allow(clippy::too_many_arguments)]
pub fn still<M: GbMem + ?Sized>(
    m: &M,
    frame: &[u8; LCD_W * LCD_H],
    palette: &[u16; 64],
    view: &mut View,
    layout: &Layout,
    faces: &mut [FaceBuf; 6],
    fb: &Fallback,
    fog_px: u8,
) -> bool {
    // Without a mask, `pick_region` only finds framed boxes.
    let active = pick_region(m, None, fb.last_typed());
    if active.is_none() && find_cursor(m, None).is_some() {
        return false;
    }
    // The backdrop: the commonest colour.
    let mut hist = [0u16; 64];
    for &p in frame.iter() {
        hist[(p & 0x3F) as usize] += 1;
    }
    let bg = (0..64).max_by_key(|&i| hist[i]).unwrap_or(0) as u8;

    let mut centre = (LCD_W as i32 / 2, LCD_H as i32 / 2);
    let hole = active.map(|(x0, y0, x1, y1)| (x0 * 8, y0 * 8, (x1 + 1) * 8, (y1 + 1) * 8));
    if let Some((hx0, hy0, hx1, hy1)) = hole {
        // Centre the up face on the picture: whatever isn't backdrop
        // outside the box, to the nearest 4 px so it doesn't shimmer.
        let (mut x0, mut y0, mut x1, mut y1) = (LCD_W, LCD_H, 0, 0);
        for y in 0..LCD_H {
            for x in 0..LCD_W {
                let in_hole = (hx0..hx1).contains(&x) && (hy0..hy1).contains(&y);
                if !in_hole && frame[y * LCD_W + x] & 0x3F != bg {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
                }
            }
        }
        if x0 <= x1 {
            centre = (
                ((x0 + x1 + 1) as i32 / 2 + 2) & !3,
                ((y0 + y1 + 1) as i32 / 2 + 2) & !3,
            );
        }
    }
    view.from_frame(frame, palette, centre);
    if let Some((hx0, hy0, hx1, hy1)) = hole {
        // The box's text goes to the front face: blank it in the fold.
        for y in hy0..hy1 {
            for x in hx0..hx1 {
                let (cx, cy) = (x as i32 - centre.0 + MID, y as i32 - centre.1 + MID);
                if (0..V as i32).contains(&cx) && (0..V as i32).contains(&cy) {
                    view.idx[cy as usize * V + cx as usize] = bg;
                }
            }
        }
    }
    view.compute_light(fog_px);
    view.drape_onto(layout, faces, 0);
    if active.is_some() {
        fallback::front_panel(m, None, layout, faces, fb);
    }
    true
}
