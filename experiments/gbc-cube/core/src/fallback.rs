//! Phase 4: screens that don't wrap.
//!
//! Text boxes, menus and battles are laid out for 160×144, and the cube's
//! faces are 64×64. Text stays at 1:1 (8×8 glyphs, the game's own font), so
//! the question is only which part goes on which face. Three layouts:
//!
//! * **A, [`UiStyle::Pan`]**: the whole frame draped over the cube like the
//!   map in Phase 2, centred on whatever is active (the text being typed,
//!   the menu cursor), panning smoothly as it moves. Side faces show what's
//!   around it. Works for any game: without RAM knowledge it follows the
//!   part of the frame that changed.
//! * **B, [`UiStyle::Spread`]**: a fixed spread. The frame's bottom 64 rows
//!   (where Game Boy games put text boxes and battle menus) wrap round the
//!   west, front (south) and east faces as one band; the up face shows the
//!   middle of the frame, the back face its top-left corner.
//! * **C, [`UiStyle::Front`]** (recommended for Crystal): the world keeps
//!   wrapping the cube and the active text box or menu moves to the front
//!   (south) face, read straight out of `wTilemap` and re-flowed to 8
//!   characters a line in the game's own font tiles. In battle the up face
//!   shows the opponent, the sides the HP boxes and your Pokémon. Screens
//!   it doesn't understand get A.
//!
//! "Front" is the south face: you hold the cube the way you'd hold a Game
//! Boy, the bottom of the map toward you.

use crate::crystal::charmap;
use crate::crystal::screen::{attrmap, tilemap, ScreenInfo, COLS, ROWS};
use crate::geom::{Compass, Layout, Role};
use crate::mem::{reg, GbMem};
use crate::ppu::{bg_tile_offset, tile_row};
use crate::view::View;
use crate::{FaceBuf, FACE, LCD_H, LCD_W};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiStyle {
    Pan,
    Spread,
    #[default]
    Front,
}

/// Where the active part of the screen is, smoothed.
#[derive(Clone)]
pub struct Fallback {
    /// Focus point in frame pixels × 16.
    focus: (i32, i32),
    prev_tilemap: [u8; COLS * ROWS],
    prev_frame: [u8; LCD_W * LCD_H],
    /// Newest tile that changed to text, (x, y) in tiles.
    last_typed: Option<(usize, usize)>,
}

impl Default for Fallback {
    fn default() -> Self {
        Self::new()
    }
}

impl Fallback {
    pub const fn new() -> Self {
        Fallback {
            focus: (72 * 16, 72 * 16),
            prev_tilemap: [0; COLS * ROWS],
            prev_frame: [0; LCD_W * LCD_H],
            last_typed: None,
        }
    }

    pub fn focus(&self) -> (i32, i32) {
        (self.focus.0 / 16, self.focus.1 / 16)
    }

    /// Note what changed this frame. `info` is `Some` for Crystal.
    pub fn observe<M: GbMem + ?Sized>(
        &mut self,
        m: &M,
        info: Option<&ScreenInfo>,
        frame: &[u8; LCD_W * LCD_H],
    ) {
        if info.is_some() {
            // Reading order: the last changed text tile is the newest
            // character the text engine printed.
            for y in 0..ROWS {
                for x in 0..COLS {
                    let t = tilemap(m, x, y);
                    let i = y * COLS + x;
                    if t != self.prev_tilemap[i] && t >= 0x80 && t != charmap::SPACE {
                        self.last_typed = Some((x, y));
                    }
                    self.prev_tilemap[i] = t;
                }
            }
        }
        self.prev_frame.copy_from_slice(frame);
    }

    /// Where A should centre: the menu cursor, else the newest text, else
    /// the middle of the UI, else (no RAM knowledge) the middle of what
    /// changed in the frame.
    fn target<M: GbMem + ?Sized>(
        &self,
        m: &M,
        info: Option<&ScreenInfo>,
        frame: &[u8; LCD_W * LCD_H],
    ) -> Option<(i32, i32)> {
        let tile_centre = |(x, y): (usize, usize)| (x as i32 * 8 + 4, y as i32 * 8 + 4);
        if let Some(info) = info {
            if let Some(c) = find_cursor(m, Some(info)) {
                return Some(tile_centre(c));
            }
            if let Some(t) = self
                .last_typed
                .filter(|&(x, y)| info.is_ui(x, y) || info.ui_tiles == 0)
            {
                return Some(tile_centre(t));
            }
            if let Some((x0, y0, x1, y1)) = ui_bounds(info) {
                return Some(((x0 + x1 + 1) as i32 * 4, (y0 + y1 + 1) as i32 * 4));
            }
            return None;
        }
        // Generic: the middle of the changed pixels.
        let (mut x0, mut y0, mut x1, mut y1) = (LCD_W, LCD_H, 0, 0);
        for y in 0..LCD_H {
            for x in 0..LCD_W {
                if frame[y * LCD_W + x] != self.prev_frame[y * LCD_W + x] {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0 <= x1).then(|| ((x0 + x1) as i32 / 2, (y0 + y1) as i32 / 2))
    }

    /// Ease the focus toward `t`, keeping the up face inside the frame.
    fn track(&mut self, t: (i32, i32)) {
        let half = FACE as i32 / 2;
        let t = (
            t.0.clamp(half, LCD_W as i32 - half) * 16,
            t.1.clamp(half, LCD_H as i32 - half) * 16,
        );
        let step = |f: &mut i32, t: i32| {
            let d = t - *f;
            *f += if d.abs() <= 16 { d } else { d / 6 + d.signum() * 8 };
        };
        step(&mut self.focus.0, t.0);
        step(&mut self.focus.1, t.1);
    }

    /// A: drape the frame around the focus.
    #[allow(clippy::too_many_arguments)]
    pub fn pan<M: GbMem + ?Sized>(
        &mut self,
        m: &M,
        info: Option<&ScreenInfo>,
        frame: &[u8; LCD_W * LCD_H],
        palette: &[u16; 64],
        view: &mut View,
        layout: &Layout,
        faces: &mut [FaceBuf; 6],
        fog_px: u8,
    ) {
        if let Some(t) = self.target(m, info, frame) {
            self.track(t);
        }
        view.from_frame(frame, palette, self.focus());
        view.compute_light(fog_px);
        view.drape_onto(layout, faces, 0);
    }
}

/// B: the fixed spread.
pub fn spread(frame: &[u8; LCD_W * LCD_H], palette: &[u16; 64], layout: &Layout, faces: &mut [FaceBuf; 6]) {
    let f = FACE as i32;
    let band = LCD_H as i32 - f; // the bottom 64 rows
    let mid = (LCD_W as i32 - f) / 2; // 48
    for (i, face) in faces.iter_mut().enumerate() {
        let origin = match layout.role[i] {
            Role::Top => (mid, 16),
            Role::Side(Compass::South) => (mid, band),
            Role::Side(Compass::East) => (mid + f, band),
            Role::Side(Compass::West) => (mid - f, band),
            Role::Side(Compass::North) => (0, 0),
            Role::Bottom => continue,
        };
        let xf = layout.xf[i];
        for v in 0..FACE {
            for u in 0..FACE {
                let (x, y) = (origin.0 + u as i32, origin.1 + v as i32);
                face[xf.index(u, v)] = if (0..LCD_W as i32).contains(&x) && (0..LCD_H as i32).contains(&y) {
                    palette[(frame[y as usize * LCD_W + x as usize] & 0x3F) as usize]
                } else {
                    0
                };
            }
        }
    }
}

/// C, battle: which part of the frame each face shows, as frame rectangles
/// `(x0, y0, x1, y1)` centred on the face with black around them. From
/// where Crystal draws them (`engine/battle/core.asm`): the opponent's
/// picture (7×7 tiles at (12, 0)) on the up face, its HUD (from (1, 0)) on
/// the back, your Pokémon's picture (6×6 at (2, 6)) on the left and your
/// HUD (from (9, 7)) on the right. The boxes along the bottom go to the
/// front face as text.
pub const BATTLE_CROPS: [(Role, (i32, i32, i32, i32)); 4] = [
    (Role::Top, (96, 0, 152, 56)),
    (Role::Side(Compass::North), (8, 0, 72, 24)),
    (Role::Side(Compass::West), (16, 48, 64, 96)),
    (Role::Side(Compass::East), (80, 56, 144, 96)),
];

pub fn battle_crops(
    frame: &[u8; LCD_W * LCD_H],
    palette: &[u16; 64],
    layout: &Layout,
    faces: &mut [FaceBuf; 6],
) {
    for (role, (x0, y0, x1, y1)) in BATTLE_CROPS {
        let face = layout.face_with(role).index();
        let xf = layout.xf[face];
        let (w, h) = (x1 - x0, y1 - y0);
        let (ox, oy) = ((FACE as i32 - w) / 2, (FACE as i32 - h) / 2);
        for v in 0..FACE {
            for u in 0..FACE {
                let (x, y) = (x0 + u as i32 - ox, y0 + v as i32 - oy);
                let inside = (x0..x1).contains(&x) && (y0..y1).contains(&y);
                faces[face][xf.index(u, v)] =
                    if inside && (0..LCD_W as i32).contains(&x) && (0..LCD_H as i32).contains(&y) {
                        palette[(frame[y as usize * LCD_W + x as usize] & 0x3F) as usize]
                    } else {
                        0
                    };
            }
        }
    }
}

/// A run of tiles to draw: the tile ID and the `wTilemap` position it came
/// from (for its attributes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Glyph {
    tile: u8,
    x: u8,
    y: u8,
}

const LINE: usize = FACE / 8;

/// Up to 16 lines of 8 glyphs.
struct Lines {
    g: [[Option<Glyph>; LINE]; 16],
    n: usize,
    cursor_line: Option<usize>,
}

impl Lines {
    fn new() -> Self {
        Lines {
            g: [[None; LINE]; 16],
            n: 0,
            cursor_line: None,
        }
    }

    fn push(&mut self, glyphs: &[Glyph]) {
        if self.n == self.g.len() || glyphs.is_empty() {
            return;
        }
        for (slot, g) in self.g[self.n].iter_mut().zip(glyphs) {
            *slot = Some(*g);
        }
        if glyphs.iter().any(|g| g.tile == charmap::CURSOR) {
            self.cursor_line = Some(self.n);
        }
        self.n += 1;
    }
}

/// The ▶ cursor, among the UI tiles if `mask` is given.
fn find_cursor<M: GbMem + ?Sized>(m: &M, mask: Option<&ScreenInfo>) -> Option<(usize, usize)> {
    (0..ROWS)
        .flat_map(|y| (0..COLS).map(move |x| (x, y)))
        .find(|&(x, y)| mask.is_none_or(|i| i.is_ui(x, y)) && tilemap(m, x, y) == charmap::CURSOR)
}

type Rect = (usize, usize, usize, usize);

/// Crystal's framed boxes on screen: from each ┌, right to the first ┐ and
/// down to the first └ (a box drawn over another, like the battle menu over
/// the text box, shares the outer one's right edge). Inclusive tile bounds.
fn find_boxes<M: GbMem + ?Sized>(m: &M, mask: Option<&ScreenInfo>, out: &mut [Rect; 8]) -> usize {
    const TL: u8 = charmap::FRAME_FIRST;
    const TR: u8 = charmap::FRAME_FIRST + 2;
    const BL: u8 = charmap::FRAME_FIRST + 4;
    let mut n = 0;
    for y in 0..ROWS {
        for x in 0..COLS {
            if n == out.len() || tilemap(m, x, y) != TL || !mask.is_none_or(|i| i.is_ui(x, y)) {
                continue;
            }
            let right = (x + 1..COLS).find(|&rx| tilemap(m, rx, y) == TR);
            let bottom = (y + 1..ROWS).find(|&by| tilemap(m, x, by) == BL);
            if let (Some(x1), Some(y1)) = (right, bottom) {
                out[n] = (x, y, x1, y1);
                n += 1;
            }
        }
    }
    n
}

fn contains(r: Rect, p: (usize, usize)) -> bool {
    (r.0..=r.2).contains(&p.0) && (r.1..=r.3).contains(&p.1)
}

/// The region C shows: the smallest box holding the cursor, else the
/// smallest holding the newest text, else the biggest box; without boxes,
/// the connected UI tiles the same way.
fn pick_region<M: GbMem + ?Sized>(
    m: &M,
    mask: Option<&ScreenInfo>,
    typed: Option<(usize, usize)>,
) -> Option<Rect> {
    let prefer = find_cursor(m, mask).or(typed);
    let mut boxes = [(0, 0, 0, 0); 8];
    let n = find_boxes(m, mask, &mut boxes);
    let area = |r: &Rect| (r.2 - r.0 + 1) * (r.3 - r.1 + 1);
    if n > 0 {
        if let Some(p) = prefer {
            if let Some(b) = boxes[..n]
                .iter()
                .filter(|b| contains(**b, p))
                .min_by_key(|b| area(b))
            {
                return Some(*b);
            }
        }
        return boxes[..n].iter().max_by_key(|b| area(b)).copied();
    }
    mask.and_then(|i| active_region(i, prefer))
}

/// Bounding box (tiles, inclusive) of the UI tiles.
fn ui_bounds(info: &ScreenInfo) -> Option<(usize, usize, usize, usize)> {
    let mut b: Option<(usize, usize, usize, usize)> = None;
    for y in 0..ROWS {
        for x in 0..COLS {
            if info.is_ui(x, y) {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    b
}

/// The UI region C shows: the connected group of UI tiles holding the
/// cursor, else the newest text, else the biggest. Inclusive tile bounds.
fn active_region(info: &ScreenInfo, prefer: Option<(usize, usize)>) -> Option<(usize, usize, usize, usize)> {
    let mut label = [0u8; COLS * ROWS];
    let mut best: Option<(Rect, usize, bool)> = None;
    let mut next = 0u8;
    for sy in 0..ROWS {
        for sx in 0..COLS {
            if !info.is_ui(sx, sy) || label[sy * COLS + sx] != 0 {
                continue;
            }
            next += 1;
            // Flood fill (4-connected) with a small explicit stack.
            let mut stack = [(0usize, 0usize); COLS * ROWS];
            let mut top = 1;
            stack[0] = (sx, sy);
            label[sy * COLS + sx] = next;
            let (mut b, mut size, mut has_pref) = ((sx, sy, sx, sy), 0, false);
            while top > 0 {
                top -= 1;
                let (x, y) = stack[top];
                size += 1;
                has_pref |= prefer == Some((x, y));
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
                let n = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in n {
                    if nx < COLS && ny < ROWS && info.is_ui(nx, ny) && label[ny * COLS + nx] == 0 {
                        label[ny * COLS + nx] = next;
                        stack[top] = (nx, ny);
                        top += 1;
                    }
                }
            }
            let better = match best {
                None => true,
                Some((_, s, p)) => (has_pref && !p) || (has_pref == p && size > s),
            };
            if better {
                best = Some((b, size, has_pref));
            }
        }
    }
    best.map(|(b, _, _)| b)
}

fn is_frame(t: u8) -> bool {
    (charmap::FRAME_FIRST..=charmap::FRAME_LAST).contains(&t)
}

/// Lay out the region's text in lines of at most 8 glyphs.
fn reflow<M: GbMem + ?Sized>(m: &M, r: (usize, usize, usize, usize)) -> Lines {
    let (mut x0, mut y0, mut x1, mut y1) = r;
    // Inside the frame, if it has one.
    if is_frame(tilemap(m, x0, y0)) && x1 > x0 + 1 && y1 > y0 + 1 {
        x0 += 1;
        y0 += 1;
        x1 -= 1;
        y1 -= 1;
    }
    let width = x1 - x0 + 1;
    let row = |y: usize| {
        (x0..=x1).map(move |x| Glyph {
            tile: tilemap(m, x, y),
            x: x as u8,
            y: y as u8,
        })
    };
    let blank = |g: &Glyph| g.tile == charmap::SPACE || is_frame(g.tile);
    let mut lines = Lines::new();
    let mut buf = [Glyph::default(); COLS];

    if width <= LINE {
        // Fits: rows as they are, minus empty ones.
        for y in y0..=y1 {
            let mut n = 0;
            for g in row(y) {
                buf[n] = g;
                n += 1;
            }
            if buf[..n].iter().any(|g| !blank(g)) {
                lines.push(&buf[..n]);
            }
        }
        return lines;
    }

    // Menu-like (a cursor, or items side by side): every item on its own
    // line. Otherwise prose: words re-wrapped.
    let menu = (y0..=y1).any(|y| {
        let mut gap = 0;
        let mut seen = false;
        for g in row(y) {
            if g.tile == charmap::CURSOR || g.tile == charmap::CURSOR_HOLLOW {
                return true;
            }
            if blank(&g) {
                gap += 1;
            } else {
                if seen && gap >= 2 {
                    return true;
                }
                seen = true;
                gap = 0;
            }
        }
        false
    });

    if menu {
        // Items line up in columns (Crystal's battle menu: FIGHT over PACK,
        // PKMN over RUN, a single blank between them that is the second
        // column's cursor slot). A column starts wherever the tile to its
        // left is blank in every row; each row's piece of each column is
        // one item, read row by row.
        let rows_used = |x: usize| {
            (y0..=y1).all(|y| {
                blank(&Glyph {
                    tile: tilemap(m, x, y),
                    x: 0,
                    y: 0,
                })
            })
        };
        let mut cols = [0usize; COLS + 1];
        let mut nc = 0;
        cols[nc] = x0;
        nc += 1;
        for x in x0 + 1..=x1 {
            let left_blank = rows_used(x - 1);
            let here_blank = rows_used(x);
            if left_blank && !here_blank && x - 1 > cols[nc - 1] {
                // Keep the blank (a cursor slot) with the item it precedes.
                cols[nc] = x - 1;
                nc += 1;
            }
        }
        cols[nc] = x1 + 1;
        for y in y0..=y1 {
            for c in 0..nc {
                let mut n = 0;
                for x in cols[c]..cols[c + 1] {
                    buf[n] = Glyph {
                        tile: tilemap(m, x, y),
                        x: x as u8,
                        y: y as u8,
                    };
                    n += 1;
                }
                // Trim: right-hand blanks, and left-hand ones beyond a single
                // cursor slot.
                while n > 0 && blank(&buf[n - 1]) {
                    n -= 1;
                }
                let mut lead = 0;
                while lead < n && blank(&buf[lead]) {
                    lead += 1;
                }
                if lead == n {
                    continue;
                }
                let from = lead.saturating_sub(1);
                lines.push(&buf[from..n.min(from + LINE)]);
            }
        }
        return lines;
    }

    // Prose.
    let mut line = [Glyph::default(); LINE];
    let mut len = 0;
    let mut word = [Glyph::default(); COLS];
    let mut wlen = 0;
    let flush_word = |line: &mut [Glyph; LINE], len: &mut usize, word: &[Glyph], lines: &mut Lines| {
        let mut w = word;
        while !w.is_empty() {
            let need = w.len() + usize::from(*len > 0);
            if *len > 0 && need > LINE - *len {
                lines.push(&line[..*len]);
                *len = 0;
                continue;
            }
            if *len > 0 {
                line[*len] = Glyph {
                    tile: charmap::SPACE,
                    ..w[0]
                };
                *len += 1;
            }
            let take = w.len().min(LINE - *len);
            line[*len..*len + take].copy_from_slice(&w[..take]);
            *len += take;
            w = &w[take..];
            if !w.is_empty() {
                lines.push(&line[..*len]);
                *len = 0;
            }
        }
    };
    for y in y0..=y1 {
        for g in row(y).chain(core::iter::once(Glyph {
            tile: charmap::SPACE,
            x: 0,
            y: 0,
        })) {
            if blank(&g) {
                if wlen > 0 {
                    flush_word(&mut line, &mut len, &word[..wlen], &mut lines);
                    wlen = 0;
                }
            } else if wlen < word.len() {
                word[wlen] = g;
                wlen += 1;
            }
        }
    }
    if len > 0 {
        lines.push(&line[..len]);
    }
    lines
}

/// C: the active text or menu, re-flowed, on the front face. Returns false
/// if there was nothing to show (the face is left as it was).
pub fn front_panel<M: GbMem + ?Sized>(
    m: &M,
    mask: Option<&ScreenInfo>,
    layout: &Layout,
    faces: &mut [FaceBuf; 6],
    fb: &Fallback,
) -> bool {
    let Some(r) = pick_region(m, mask, fb.last_typed) else {
        return false;
    };
    let lines = reflow(m, r);
    if lines.n == 0 {
        return false;
    }
    let first = match lines.cursor_line {
        Some(c) if lines.n > LINE => c.saturating_sub(LINE / 2).min(lines.n - LINE),
        _ => lines.n.saturating_sub(LINE),
    };
    let shown = lines.n - first;
    let face = layout.face_with(Role::Side(Compass::South)).index();
    let xf = layout.xf[face];
    let lcdc = m.reg(reg::LCDC);
    let vram = m.vram();
    // Background: colour 0 of the first glyph's palette, like the text box.
    let first_glyph = lines.g[first]
        .iter()
        .flatten()
        .next()
        .copied()
        .unwrap_or_default();
    let bg_pal = attrmap(m, first_glyph.x as usize, first_glyph.y as usize) & 7;
    let colour = |pal: u8, c: u8| {
        let i = (pal as usize * 4 + c as usize) * 2;
        let p = m.bg_palette();
        crate::color::bgr555_to_rgb565(u16::from_le_bytes([p[i], p[i + 1]]))
    };
    let paper = colour(bg_pal, 0);
    for v in 0..FACE {
        for u in 0..FACE {
            faces[face][xf.index(u, v)] = paper;
        }
    }
    // Vertically centred when there are few lines.
    let top = (LINE - shown.min(LINE)) / 2;
    for (li, line) in lines.g[first..lines.n].iter().take(LINE).enumerate() {
        // Left-aligned, as the game sets text.
        for (ci, g) in line.iter().enumerate() {
            let Some(g) = g else { continue };
            let a = attrmap(m, g.x as usize, g.y as usize);
            let off = bg_tile_offset(lcdc, g.tile, a);
            for r in 0..8 {
                let px = tile_row(vram, off, r, a, 8);
                for (i, &c) in px.iter().enumerate() {
                    let (u, v) = (ci * 8 + i, (top + li) * 8 + r);
                    faces[face][xf.index(u, v)] = colour(a & 7, c);
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crystal::syms;

    /// Just enough memory for `wTilemap` tests.
    struct Ram {
        wram: [u8; 0x8000],
        vram: [u8; 0x4000],
        io: [u8; 0x100],
        pal: [u8; 64],
    }

    impl GbMem for Ram {
        fn wram(&self) -> &[u8] {
            &self.wram
        }
        fn vram(&self) -> &[u8] {
            &self.vram
        }
        fn oam(&self) -> &[u8] {
            &[]
        }
        fn io(&self) -> &[u8] {
            &self.io
        }
        fn bg_palette(&self) -> &[u8] {
            &self.pal
        }
        fn obj_palette(&self) -> &[u8] {
            &self.pal
        }
        fn rom(&self, _: u32) -> u8 {
            0
        }
    }

    fn put(r: &mut Ram, x: usize, y: usize, s: &[u8]) {
        let base = syms::W_TILEMAP.addr as usize - 0xC000 + y * COLS + x;
        r.wram[base..base + s.len()].copy_from_slice(s);
    }

    fn text(lines: &Lines) -> std::vec::Vec<std::string::String> {
        (0..lines.n)
            .map(|i| {
                lines.g[i]
                    .iter()
                    .flatten()
                    .map(|g| match g.tile {
                        charmap::SPACE => ' ',
                        charmap::CURSOR => '>',
                        t if (0x80..0x80 + 26).contains(&t) => (b'A' + t - 0x80) as char,
                        _ => '?',
                    })
                    .collect()
            })
            .collect()
    }

    fn enc(s: &str) -> std::vec::Vec<u8> {
        s.bytes()
            .map(|b| match b {
                b' ' => charmap::SPACE,
                b'>' => charmap::CURSOR,
                b'[' => 0x79,
                b']' => 0x7B,
                b'|' => 0x7C,
                b'{' => 0x7D,
                b'}' => 0x7E,
                b'-' => 0x7A,
                c => 0x80 + (c - b'A'),
            })
            .collect()
    }

    fn ram() -> Ram {
        let mut r = Ram {
            wram: [0; 0x8000],
            vram: [0; 0x4000],
            io: [0; 0x100],
            pal: [0; 64],
        };
        for y in 0..ROWS {
            put(&mut r, 0, y, &[charmap::SPACE; COLS]);
        }
        r
    }

    #[test]
    fn prose_rewraps_to_eight_columns() {
        let mut r = ram();
        put(&mut r, 0, 12, &enc("[------------------]"));
        put(&mut r, 0, 13, &enc("|                  |"));
        put(&mut r, 0, 14, &enc("| HELLO THERE WELCO|"));
        put(&mut r, 0, 15, &enc("|                  |"));
        put(&mut r, 0, 16, &enc("| ME TO THE WORLD  |"));
        put(&mut r, 0, 17, &enc("{------------------}"));
        let lines = reflow(&r, (0, 12, 19, 17));
        // Words split across the box's lines stay split (the game hyphenates
        // with spaces, not us); everything fits 8 wide.
        assert_eq!(text(&lines), ["HELLO", "THERE", "WELCO ME", "TO THE", "WORLD"]);
    }

    #[test]
    fn menus_put_each_item_on_a_line() {
        // Crystal's battle menu: a single blank (the next item's cursor
        // slot) between columns, items aligned under each other.
        let mut r = ram();
        put(&mut r, 7, 12, &enc("[-----------]"));
        put(&mut r, 7, 13, &enc("|           |"));
        put(&mut r, 7, 14, &enc("|>FIGHT PKMN|"));
        put(&mut r, 7, 15, &enc("|           |"));
        put(&mut r, 7, 16, &enc("| PACK  RUN |"));
        put(&mut r, 7, 17, &enc("{-----------}"));
        let lines = reflow(&r, (7, 12, 19, 17));
        assert_eq!(text(&lines), [">FIGHT", " PKMN", " PACK", " RUN"]);
        assert_eq!(lines.cursor_line, Some(0));
    }
}
