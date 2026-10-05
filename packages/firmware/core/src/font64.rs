//! Bitmap fonts and pixel sprites for the 64×64 panel.
//!
//! At 0.168 mm a pixel, the 96×96 die's anti-aliased Space Grotesk (sampled
//! down from its 96×96 cuts) turns to mush below about 10 px. These fonts
//! are drawn for the grid instead, one bit a pixel, no smoothing:
//!
//! * [`TEXT`] 5×7 (lowercase x-height 5, descenders 2): **the smallest size
//!   anything a player has to read is set in.** Labels, menu text, words.
//! * [`TAG`] 3×5, capitals and digits only: tags that repeat something shown
//!   elsewhere or that nobody has to read to play (a face number, `MAX`,
//!   the status bar). Never the only place information appears.
//! * The result numerals ([`crate::numerals64`]) at 44, 30 and 22 px.
//!
//! Glyphs are kept as pixel art in this file (`#` lit, `.` dark, rows
//! separated by whitespace) and parsed at compile time, so changing one is
//! editing the picture. Fonts are proportional: a glyph is as wide as its
//! rows.
//!
//! Text is drawn through the painter's transform in pixel units
//! ([`crate::gfx::Transform::in_pixels`]), so it turns with the face like
//! everything else and lands exactly on the grid at the four quarter turns.

use smokebomb_hal::Target;

use crate::gfx::{Painter, Style};

/// One glyph: up to 32 pixels wide (bit 31 is the left column), `H` rows.
#[derive(Clone, Copy, Debug)]
pub struct Glyph<const H: usize> {
    pub ch: char,
    pub width: u8,
    pub rows: [u32; H],
}

/// Parse pixel art: cells equal to `key` are lit, any other cell dark;
/// whitespace separates rows. Panics (at compile time) on ragged rows or
/// more than `H` rows.
pub const fn layer<const H: usize>(ch: char, art: &str, key: u8) -> Glyph<H> {
    let b = art.as_bytes();
    let mut rows = [0u32; H];
    let (mut i, mut row, mut col, mut width) = (0, 0, 0usize, 0usize);
    while i <= b.len() {
        let end = i == b.len() || b[i] == b' ' || b[i] == b'\n';
        if end {
            if col > 0 {
                if width == 0 {
                    width = col;
                } else if col != width {
                    panic!("ragged glyph rows");
                }
                row += 1;
                col = 0;
            }
        } else {
            if row >= H {
                panic!("glyph taller than its font");
            }
            if col >= 32 {
                panic!("glyph wider than 32");
            }
            if b[i] == key {
                rows[row] |= 0x8000_0000 >> col;
            }
            col += 1;
        }
        i += 1;
    }
    Glyph {
        ch,
        width: width as u8,
        rows,
    }
}

/// A one-colour glyph: `#` lit.
pub const fn glyph<const H: usize>(ch: char, art: &str) -> Glyph<H> {
    layer(ch, art, b'#')
}

/// Horizontal alignment about the given x.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// A bitmap font: its glyphs, the height of its box (cap height plus
/// descender rows), the gap between glyphs, and whether lowercase falls
/// back to capitals.
pub struct BitFont<const H: usize> {
    pub glyphs: &'static [Glyph<H>],
    /// Rows from the top of the box to the baseline (the cap height).
    pub cap: u8,
    pub gap: u8,
    pub caps_only: bool,
}

impl<const H: usize> BitFont<H> {
    pub fn find(&self, ch: char) -> Option<&Glyph<H>> {
        let ch = if self.caps_only {
            ch.to_ascii_uppercase()
        } else {
            ch
        };
        self.glyphs.iter().find(|g| g.ch == ch)
    }

    /// Width of `text` in pixels at scale 1 (unknown characters are
    /// skipped).
    pub fn measure(&self, text: &str) -> usize {
        let mut w = 0usize;
        let mut n = 0usize;
        for ch in text.chars() {
            if let Some(g) = self.find(ch) {
                w += g.width as usize;
                n += 1;
            }
        }
        w + n.saturating_sub(1) * self.gap as usize
    }

    /// Whether every character of `text` has a glyph.
    pub fn covers(&self, text: &str) -> bool {
        text.chars().all(|c| self.find(c).is_some())
    }

    /// Draw `text` with the top of its box at `y` and aligned on `x`, each
    /// font pixel `scale` units (pixel units: see the module docs). One
    /// draw call, so `style`'s alpha applies once.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<T: Target>(
        &self,
        p: &mut Painter<T>,
        text: &str,
        x: f32,
        y: f32,
        scale: f32,
        align: Align,
        style: Style,
    ) {
        let w = self.measure(text) as f32 * scale;
        let shown = text
            .chars()
            .filter(|c| !c.is_whitespace() && self.find(*c).is_some())
            .count();
        p.note_text(self.cap as f32 * scale, w, shown, style.alpha);
        let mut pen = match align {
            Align::Left => x,
            Align::Center => x - libm::floorf(w / scale / 2.0) * scale,
            Align::Right => x - w,
        };
        p.begin();
        for ch in text.chars() {
            if let Some(g) = self.find(ch) {
                p.cover_rows(&g.rows, g.width as usize, pen, y, scale);
                pen += (g.width + self.gap) as f32 * scale;
            }
        }
        p.finish(style);
    }
}

/// Draw one glyph (or sprite layer) with its top-left at `(x, y)`.
pub fn draw_glyph<T: Target, const H: usize>(
    p: &mut Painter<T>,
    g: &Glyph<H>,
    x: f32,
    y: f32,
    scale: f32,
    style: Style,
) {
    p.begin();
    p.cover_rows(&g.rows, g.width as usize, x, y, scale);
    p.finish(style);
}

/// Draw one glyph centred on `(cx, cy)`, snapped to whole units so it stays
/// on the pixel grid.
pub fn draw_centered<T: Target, const H: usize>(
    p: &mut Painter<T>,
    g: &Glyph<H>,
    cx: f32,
    cy: f32,
    scale: f32,
    style: Style,
) {
    let h = g.rows.iter().rposition(|&r| r != 0).map_or(0, |i| i + 1);
    let x = cx - libm::floorf(g.width as f32 / 2.0) * scale;
    let y = cy - libm::floorf(h as f32 / 2.0) * scale;
    draw_glyph(p, g, x, y, scale, style);
}

// ---------- 5×7 text ----------

/// 5×7 text: capitals 7 rows, lowercase x-height 5, descenders 2 below the
/// baseline (9 rows in all). Hand-drawn.
pub static TEXT: BitFont<9> = BitFont {
    glyphs: &TEXT_GLYPHS,
    cap: 7,
    gap: 1,
    caps_only: false,
};

static TEXT_GLYPHS: [Glyph<9>; 99] = [
    glyph(' ', "... ... ... ... ... ... ..."),
    glyph('!', "# # # # # . #"),
    glyph('"', "#.# #.# ... ... ... ... ..."),
    glyph('#', ".#.#. .#.#. ##### .#.#. ##### .#.#. .#.#."),
    glyph('$', "..#.. .#### #.#.. .###. ..#.# ####. ..#.."),
    glyph('%', "##..# ##..# ...#. ..#.. .#... #..## #..##"),
    glyph('&', ".##.. #..#. #.#.. .#... #.#.# #..#. .##.#"),
    glyph('\'', "# # . . . . ."),
    glyph('(', "..# .#. #.. #.. #.. .#. ..#"),
    glyph(')', "#.. .#. ..# ..# ..# .#. #.."),
    glyph('*', "..... ..#.. #.#.# .###. #.#.# ..#.. ....."),
    glyph('+', "..... ..#.. ..#.. ##### ..#.. ..#.. ....."),
    glyph(',', ".. .. .. .. .. .# .# #."),
    glyph('-', "..... ..... ..... ##### ..... ..... ....."),
    glyph('.', ". . . . . . #"),
    glyph('/', "....# ....# ...#. ..#.. .#... #.... #...."),
    glyph('0', ".###. #...# #..## #.#.# ##..# #...# .###."),
    glyph('1', "..#.. .##.. ..#.. ..#.. ..#.. ..#.. .###."),
    glyph('2', ".###. #...# ....# ...#. ..#.. .#... #####"),
    glyph('3', "##### ...#. ..#.. ...#. ....# #...# .###."),
    glyph('4', "...#. ..##. .#.#. #..#. ##### ...#. ...#."),
    glyph('5', "##### #.... ####. ....# ....# #...# .###."),
    glyph('6', "..##. .#... #.... ####. #...# #...# .###."),
    glyph('7', "##### ....# ...#. ..#.. .#... .#... .#..."),
    glyph('8', ".###. #...# #...# .###. #...# #...# .###."),
    glyph('9', ".###. #...# #...# .#### ....# ...#. .##.."),
    glyph(':', ". . # . . # ."),
    glyph(';', ".. .. .# .. .. .# .# #."),
    glyph('<', "...# ..#. .#.. #... .#.. ..#. ...#"),
    glyph('=', "..... ..... ##### ..... ##### ..... ....."),
    glyph('>', "#... .#.. ..#. ...# ..#. .#.. #..."),
    glyph('?', ".###. #...# ....# ...#. ..#.. ..... ..#.."),
    glyph('@', ".###. #...# #.### #.#.# #.### #.... .###."),
    glyph('A', ".###. #...# #...# ##### #...# #...# #...#"),
    glyph('B', "####. #...# #...# ####. #...# #...# ####."),
    glyph('C', ".###. #...# #.... #.... #.... #...# .###."),
    glyph('D', "####. #...# #...# #...# #...# #...# ####."),
    glyph('E', "##### #.... #.... ####. #.... #.... #####"),
    glyph('F', "##### #.... #.... ####. #.... #.... #...."),
    glyph('G', ".###. #...# #.... #.### #...# #...# .####"),
    glyph('H', "#...# #...# #...# ##### #...# #...# #...#"),
    glyph('I', "### .#. .#. .#. .#. .#. ###"),
    glyph('J', "..### ...#. ...#. ...#. ...#. #..#. .##.."),
    glyph('K', "#...# #..#. #.#.. ##... #.#.. #..#. #...#"),
    glyph('L', "#.... #.... #.... #.... #.... #.... #####"),
    glyph('M', "#...# ##.## #.#.# #.#.# #...# #...# #...#"),
    glyph('N', "#...# #...# ##..# #.#.# #..## #...# #...#"),
    glyph('O', ".###. #...# #...# #...# #...# #...# .###."),
    glyph('P', "####. #...# #...# ####. #.... #.... #...."),
    glyph('Q', ".###. #...# #...# #...# #.#.# #..#. .##.#"),
    glyph('R', "####. #...# #...# ####. #.#.. #..#. #...#"),
    glyph('S', ".###. #...# #.... .###. ....# #...# .###."),
    glyph('T', "##### ..#.. ..#.. ..#.. ..#.. ..#.. ..#.."),
    glyph('U', "#...# #...# #...# #...# #...# #...# .###."),
    glyph('V', "#...# #...# #...# #...# #...# .#.#. ..#.."),
    glyph('W', "#...# #...# #...# #.#.# #.#.# #.#.# .#.#."),
    glyph('X', "#...# #...# .#.#. ..#.. .#.#. #...# #...#"),
    glyph('Y', "#...# #...# .#.#. ..#.. ..#.. ..#.. ..#.."),
    glyph('Z', "##### ....# ...#. ..#.. .#... #.... #####"),
    glyph('[', "### #.. #.. #.. #.. #.. ###"),
    glyph('\\', "#.... #.... .#... ..#.. ...#. ....# ....#"),
    glyph(']', "### ..# ..# ..# ..# ..# ###"),
    glyph('^', "..#.. .#.#. #...# ..... ..... ..... ....."),
    glyph('_', "..... ..... ..... ..... ..... ..... #####"),
    glyph('`', "#. .# .. .. .. .. .."),
    glyph('a', "..... ..... .###. ....# .#### #...# .####"),
    glyph('b', "#.... #.... #.##. ##..# #...# #...# ####."),
    glyph('c', "..... ..... .###. #.... #.... #...# .###."),
    glyph('d', "....# ....# .##.# #..## #...# #...# .####"),
    glyph('e', "..... ..... .###. #...# ##### #.... .###."),
    glyph('f', "..## .#.. .#.. ###. .#.. .#.. .#.."),
    glyph('g', "..... ..... .#### #...# #...# #...# .#### ....# .###."),
    glyph('h', "#.... #.... #.##. ##..# #...# #...# #...#"),
    glyph('i', ".#. ... ##. .#. .#. .#. ###"),
    glyph('j', "...# .... ..## ...# ...# ...# ...# #..# .##."),
    glyph('k', "#... #... #..# #.#. ##.. #.#. #..#"),
    glyph('l', "##. .#. .#. .#. .#. .#. ###"),
    glyph('m', "..... ..... ##.#. #.#.# #.#.# #.#.# #.#.#"),
    glyph('n', "..... ..... #.##. ##..# #...# #...# #...#"),
    glyph('o', "..... ..... .###. #...# #...# #...# .###."),
    glyph('p', "..... ..... ####. #...# #...# #...# ####. #.... #...."),
    glyph('q', "..... ..... .#### #...# #...# #...# .#### ....# ....#"),
    glyph('r', "..... ..... #.##. ##..# #.... #.... #...."),
    glyph('s', "..... ..... .###. #.... .###. ....# ####."),
    glyph('t', ".#.. .#.. ###. .#.. .#.. .#.# ..#."),
    glyph('u', "..... ..... #...# #...# #...# #..## .##.#"),
    glyph('v', "..... ..... #...# #...# #...# .#.#. ..#.."),
    glyph('w', "..... ..... #...# #...# #.#.# #.#.# .#.#."),
    glyph('x', "..... ..... #...# .#.#. ..#.. .#.#. #...#"),
    glyph('y', "..... ..... #...# #...# #...# #...# .#### ....# .###."),
    glyph('z', "..... ..... ##### ...#. ..#.. .#... #####"),
    glyph('{', "..# .#. .#. #.. .#. .#. ..#"),
    glyph('|', "# # # # # # #"),
    glyph('}', "#.. .#. .#. ..# .#. .#. #.."),
    glyph('~', "..... ..... .#... #.#.# ...#. ..... ....."),
    glyph('×', "..... #...# .#.#. ..#.. .#.#. #...# ....."),
    glyph('·', ". . . # . . ."),
    glyph('▲', "..... ..#.. .###. ##### ..... ..... ....."),
    glyph('▼', "..... ..... ##### .###. ..#.. ..... ....."),
];

// ---------- 3×5 tags ----------

/// 3×5 capitals and digits for tags (see the module docs for when).
/// Hand-drawn. Lowercase is drawn as capitals.
pub static TAG: BitFont<5> = BitFont {
    glyphs: &TAG_GLYPHS,
    cap: 5,
    gap: 1,
    caps_only: true,
};

static TAG_GLYPHS: [Glyph<5>; 47] = [
    glyph(' ', ".. .. .. .. .."),
    glyph('0', "### #.# #.# #.# ###"),
    glyph('1', ".#. ##. .#. .#. ###"),
    glyph('2', "### ..# ### #.. ###"),
    glyph('3', "### ..# .## ..# ###"),
    glyph('4', "#.# #.# ### ..# ..#"),
    glyph('5', "### #.. ### ..# ###"),
    glyph('6', "### #.. ### #.# ###"),
    glyph('7', "### ..# .#. .#. .#."),
    glyph('8', "### #.# ### #.# ###"),
    glyph('9', "### #.# ### ..# ###"),
    glyph('A', ".#. #.# ### #.# #.#"),
    glyph('B', "##. #.# ##. #.# ##."),
    glyph('C', ".## #.. #.. #.. .##"),
    glyph('D', "##. #.# #.# #.# ##."),
    glyph('E', "### #.. ##. #.. ###"),
    glyph('F', "### #.. ##. #.. #.."),
    glyph('G', ".## #.. #.# #.# .##"),
    glyph('H', "#.# #.# ### #.# #.#"),
    glyph('I', "### .#. .#. .#. ###"),
    glyph('J', "..# ..# ..# #.# .#."),
    glyph('K', "#.# #.# ##. #.# #.#"),
    glyph('L', "#.. #.. #.. #.. ###"),
    glyph('M', "#.# ### ### #.# #.#"),
    glyph('N', "##. #.# #.# #.# #.#"),
    glyph('O', ".#. #.# #.# #.# .#."),
    glyph('P', "##. #.# ##. #.. #.."),
    glyph('Q', ".#. #.# #.# ##. .##"),
    glyph('R', "##. #.# ##. #.# #.#"),
    glyph('S', ".## #.. .#. ..# ##."),
    glyph('T', "### .#. .#. .#. .#."),
    glyph('U', "#.# #.# #.# #.# ###"),
    glyph('V', "#.# #.# #.# #.# .#."),
    glyph('W', "#.# #.# ### ### #.#"),
    glyph('X', "#.# #.# .#. #.# #.#"),
    glyph('Y', "#.# #.# .#. .#. .#."),
    glyph('Z', "### ..# .#. #.. ###"),
    glyph('+', "... .#. ### .#. ..."),
    glyph('-', "... ... ### ... ..."),
    glyph('=', "... ### ... ### ..."),
    glyph(':', ". # . # ."),
    glyph('.', ". . . . #"),
    glyph('/', "..# ..# .#. #.. #.."),
    glyph('%', "#.# ..# .#. #.. #.#"),
    glyph('×', "... #.# .#. #.# ..."),
    glyph('·', ". . # . ."),
    glyph('!', "# # # . #"),
];

// ---------- the result numerals ----------

/// A numeral size: its glyphs, the gap between digits and its height.
pub struct Numerals<const H: usize> {
    pub digits: &'static [Glyph<H>; 10],
    pub gap: u8,
}

impl<const H: usize> Numerals<H> {
    pub const HEIGHT: usize = H;

    /// How tall the digits' ink is (their cap height).
    pub fn digit_height(&self) -> usize {
        self.digits[8].rows.iter().filter(|&&r| r != 0).count()
    }

    pub fn digit(&self, ch: char) -> Option<&Glyph<H>> {
        ch.to_digit(10).map(|d| &self.digits[d as usize])
    }

    pub fn measure(&self, text: &str) -> usize {
        let n = text.chars().filter(|c| c.is_ascii_digit()).count();
        text.chars()
            .filter_map(|c| self.digit(c))
            .map(|g| g.width as usize)
            .sum::<usize>()
            + n.saturating_sub(1) * self.gap as usize
    }

    /// Draw the digits of `text` centred on `cx`, top at `y`.
    pub fn draw<T: Target>(&self, p: &mut Painter<T>, text: &str, cx: f32, y: f32, style: Style) {
        let digits = text.chars().filter(|c| c.is_ascii_digit()).count();
        p.note_text(
            self.digit_height() as f32,
            self.measure(text) as f32,
            digits,
            style.alpha,
        );
        let mut pen = cx - (self.measure(text) / 2) as f32;
        p.begin();
        for g in text.chars().filter_map(|c| self.digit(c)) {
            p.cover_rows(&g.rows, g.width as usize, pen, y, 1.0);
            pen += (g.width + self.gap) as f32;
        }
        p.finish(style);
    }
}

/// 44 px numerals, for 1–2 digits.
pub static NUM_L: Numerals<44> = Numerals {
    digits: &crate::numerals64::LARGE,
    gap: 3,
};
/// 30 px numerals, for 3 digits.
pub static NUM_M: Numerals<30> = Numerals {
    digits: &crate::numerals64::MEDIUM,
    gap: 2,
};
/// 22 px numerals, for 4 digits.
pub static NUM_S: Numerals<22> = Numerals {
    digits: &crate::numerals64::SMALL,
    gap: 2,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyphs_parse_to_their_pictures() {
        let a = TEXT.find('A').unwrap();
        assert_eq!(a.width, 5);
        assert_eq!(a.rows[0], 0b01110 << 27);
        assert_eq!(a.rows[3], 0b11111 << 27);
        assert_eq!(a.rows[7], 0, "no descender");
        let g = TEXT.find('g').unwrap();
        assert_ne!(g.rows[8], 0, "descender");
        assert_eq!(TEXT.find('i').unwrap().width, 3);
    }

    #[test]
    fn every_printable_ascii_character_has_a_5x7_glyph() {
        for c in ' '..='~' {
            assert!(TEXT.find(c).is_some(), "{c:?}");
        }
        for c in ['×', '·', '▲', '▼'] {
            assert!(TEXT.find(c).is_some(), "{c:?}");
        }
    }

    #[test]
    fn glyphs_stay_in_their_boxes() {
        for g in TEXT_GLYPHS.iter() {
            assert!((1..=5).contains(&g.width), "{:?}", g.ch);
            // Only the descender letters reach below the baseline.
            let below = g.rows[7] | g.rows[8];
            assert_eq!(below != 0, "gjpqy,;".contains(g.ch), "{:?}", g.ch);
        }
        for g in TAG_GLYPHS.iter() {
            assert!(g.width <= 3, "{:?}", g.ch);
        }
    }

    #[test]
    fn tags_cover_digits_and_capitals_and_read_lowercase_as_capitals() {
        for c in ('0'..='9').chain('A'..='Z') {
            assert!(TAG.find(c).is_some(), "{c:?}");
        }
        assert_eq!(TAG.find('m').unwrap().ch, 'M');
    }

    #[test]
    fn measure_counts_widths_and_gaps() {
        assert_eq!(TEXT.measure("d20"), 5 + 1 + 5 + 1 + 5);
        assert_eq!(TEXT.measure("il"), 3 + 1 + 3);
        assert_eq!(TAG.measure("MAX"), 11);
        assert_eq!(TEXT.measure(""), 0);
    }

    #[test]
    fn numerals_fit_the_panel() {
        // Two big digits, three medium, four small: each inside 64 px with
        // room to spare for the glass's corners.
        assert!(NUM_L.measure("88") <= 58, "{}", NUM_L.measure("88"));
        assert!(NUM_M.measure("888") <= 60, "{}", NUM_M.measure("888"));
        assert!(NUM_S.measure("8888") <= 60, "{}", NUM_S.measure("8888"));
        for (i, g) in NUM_L.digits.iter().enumerate() {
            assert_eq!(g.ch, char::from(b'0' + i as u8));
            assert_eq!(g.width, 26);
        }
        assert!(NUM_M.digits.iter().all(|g| g.width == 18));
        assert!(NUM_S.digits.iter().all(|g| g.width == 13));
    }
}
