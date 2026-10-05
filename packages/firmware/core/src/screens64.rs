//! The 64×64 colour screens, designed for the 30 mm die's 0.6" panel: not
//! the 96×96 layouts scaled down.
//!
//! Everything here is laid out in panel pixels around the face centre (x
//! right, y down, the panel spanning −32..32), with bitmap text
//! ([`crate::font64`]), hand-placed sprites ([`crate::sprites64`]) and the
//! palette in [`crate::palette64`]. The glass rounds the lit area's corners
//! by ≈10.7 px, so nothing wide sits in the top or bottom 6 rows.
//!
//! Colour carries what the 96×96 layout said with words or space: a gold
//! number is a max, a red one a fumble; the die's icon and the menu's
//! arrows are violet; charging and "saved" are mint.

use core::fmt::Write as _;

use heapless::{String, Vec};
use libm::{floorf, roundf, sinf};
use smokebomb_hal::{AssetStore, Color, Target};
use smokebomb_shared::{DieKind, RollRecord};

use crate::font64::{draw_centered, draw_glyph, Align, BitFont, Glyph, NUM_L, NUM_M, NUM_S, TAG, TEXT};
use crate::gfx::{Painter, Style};
use crate::menu::{Draft, Page, Setup};
use crate::nest::{ChargeView, Label, NestFace, Screen};
use crate::palette64 as pal;
use crate::screens::{self, setup_label, Ctx, BOOT_FADE, BOOT_STEP, BOOT_STEPS, FACE_START, LOOP_END};
use crate::smoke::Special;
use crate::sprites64 as spr;

/// The widest a line may be: the panel less a pixel or so each side, so
/// nothing touches the glass's ink border.
pub const LINE_MAX: usize = 58;
/// 5×7 line pitch: 7 rows, 2 between lines.
pub const LINE_PITCH: f32 = 9.0;

fn style(c: Color, alpha: f32) -> Style {
    Style::color(c, alpha, 0.0)
}

/// Draw in pixel units, then put the transform back.
fn in_pixels<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, draw: impl FnOnce(&mut Painter<T>)) {
    let base = c.painter.xf;
    c.painter.xf = base.in_pixels();
    draw(c.painter);
    c.painter.xf = base;
}

/// Runs of text in different colours, set as one line centred on `cx`.
fn draw_runs<T: Target, const H: usize>(
    p: &mut Painter<T>,
    font: &BitFont<H>,
    runs: &[(&str, Color)],
    cx: f32,
    y: f32,
    scale: f32,
    alpha: f32,
) {
    let widths = runs.iter().map(|(t, _)| font.measure(t));
    let total = widths.clone().sum::<usize>() + runs.len().saturating_sub(1) * font.gap as usize;
    let mut pen = cx - floorf(total as f32 / 2.0) * scale;
    for ((text, colour), w) in runs.iter().zip(widths) {
        font.draw(p, text, pen, y, scale, Align::Left, style(*colour, alpha));
        pen += (w + font.gap as usize) as f32 * scale;
    }
}

/// Split `text` at spaces into lines no wider than `max` pixels (a word
/// wider than a line is cut). At most `N` lines; returns whether it all fit.
pub fn wrap<'a, const N: usize, const H: usize>(
    font: &BitFont<H>,
    text: &'a str,
    max: usize,
    lines: &mut Vec<&'a str, N>,
) -> bool {
    lines.clear();
    let mut rest = text.trim();
    while !rest.is_empty() {
        // The longest prefix that fits, ending at a space if one does.
        let mut end = 0;
        let mut last_space = None;
        for (i, ch) in rest.char_indices() {
            let next = i + ch.len_utf8();
            if font.measure(&rest[..next]) > max {
                break;
            }
            end = next;
            if ch == ' ' {
                last_space = Some(i);
            }
        }
        if end == 0 {
            // Not even one character fits.
            return false;
        }
        // Whole words: cut at the end if a word ends there, else at the
        // last space, else (one long word) wherever it stopped fitting.
        let cut = if end == rest.len() || rest[end..].starts_with(' ') {
            end
        } else {
            last_space.filter(|&s| s > 0).unwrap_or(end)
        };
        if lines.push(rest[..cut].trim_end()).is_err() {
            return false;
        }
        rest = rest[cut..].trim_start();
    }
    true
}

/// Draw wrapped 5×7 lines centred on `cx`, the first line's top at `y`.
/// Returns how many lines it took.
fn draw_lines<T: Target>(
    p: &mut Painter<T>,
    text: &str,
    cx: f32,
    y: f32,
    colour: Color,
    alpha: f32,
) -> usize {
    let mut lines: Vec<&str, 6> = Vec::new();
    wrap(&TEXT, text, LINE_MAX, &mut lines);
    for (i, line) in lines.iter().enumerate() {
        TEXT.draw(
            p,
            line,
            cx,
            y + i as f32 * LINE_PITCH,
            1.0,
            Align::Center,
            style(colour, alpha),
        );
    }
    lines.len()
}

/// The die's icon.
pub fn die_icon(die: DieKind) -> &'static Glyph<19> {
    match die {
        DieKind::D4 => &spr::D4,
        DieKind::D6 | DieKind::PassThePot => &spr::D6,
        DieKind::D8 => &spr::D8,
        DieKind::D10 | DieKind::D100 => &spr::D10,
        DieKind::D12 => &spr::D12,
        DieKind::D20 => &spr::D20,
    }
}

// ---------- result (SIM_SPEC C6) ----------

/// The numeral size for a total of `digits` digits, and whether a parts
/// line is above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumSize {
    /// 44 px: one or two digits on their own.
    Large,
    /// 30 px: three digits, or up to three under a parts line.
    Medium,
    /// 22 px: four digits.
    Small,
}

pub fn num_size(digits: usize, parts: bool) -> NumSize {
    match (digits, parts) {
        (0..=2, false) => NumSize::Large,
        (0..=3, _) => NumSize::Medium,
        _ => NumSize::Small,
    }
}

impl NumSize {
    pub fn height(self) -> usize {
        match self {
            NumSize::Large => 44,
            NumSize::Medium => 30,
            NumSize::Small => 22,
        }
    }

    pub fn measure(self, text: &str) -> usize {
        match self {
            NumSize::Large => NUM_L.measure(text),
            NumSize::Medium => NUM_M.measure(text),
            NumSize::Small => NUM_S.measure(text),
        }
    }

    fn draw<T: Target>(self, p: &mut Painter<T>, text: &str, y: f32, s: Style) {
        match self {
            NumSize::Large => NUM_L.draw(p, text, 0.0, y, s),
            NumSize::Medium => NUM_M.draw(p, text, 0.0, y, s),
            NumSize::Small => NUM_S.draw(p, text, 0.0, y, s),
        }
    }
}

/// The parts line of a pool (`3+5+2`) and the font it fits in: 5×7 if it
/// fits a line, else 3×5 tags, else none (the total and the setup say
/// enough).
pub fn parts_line(values: &[u8]) -> Option<(String<48>, bool)> {
    if values.len() < 2 {
        return None;
    }
    let mut s: String<48> = String::new();
    for (i, v) in values.iter().enumerate() {
        let _ = write!(s, "{}{v}", if i > 0 { "+" } else { "" });
    }
    if TEXT.measure(&s) <= LINE_MAX {
        Some((s, false))
    } else if TAG.measure(&s) <= LINE_MAX {
        Some((s, true))
    } else {
        None
    }
}

/// A roll's result. One or two digits fill most of the face; the setup
/// sits under it in grey. A max turns the number gold with a gold `MAX`
/// tag; a fumble (every die a 1) turns it red with `DUD`. Pass the Pot keeps
/// the 96×96 layout for now.
pub fn draw_result<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    record: &RollRecord,
    special: Option<Special>,
    alpha: f32,
) {
    if !record.die.is_numeric() {
        screens::draw_result(c, record, special, alpha);
        return;
    }
    let mut total: String<8> = String::new();
    let _ = write!(total, "{}", record.total());
    let label = setup_label(record.die, record.values.len() as u8);
    let parts = parts_line(&record.values);
    let size = num_size(total.len(), parts.is_some());
    let (colour, tag) = match special {
        Some(Special::Max) => (pal::GOLD, Some(("MAX", pal::GOLD))),
        Some(Special::Dud) => (pal::RED, Some(("DUD", pal::RED))),
        None => (pal::WHITE, None),
    };
    let parts_h = parts
        .as_ref()
        .map_or(0, |(_, tag)| if *tag { 5 + 4 } else { 7 + 4 });
    let block = parts_h + size.height() + 4 + 7;
    let top = -floorf(block as f32 / 2.0);
    in_pixels(c, |p| {
        if let Some((s, tag)) = &parts {
            let font_y = top;
            if *tag {
                TAG.draw(p, s, 0.0, font_y, 1.0, Align::Center, style(pal::DIM, alpha));
            } else {
                TEXT.draw(p, s, 0.0, font_y, 1.0, Align::Center, style(pal::DIM, alpha));
            }
        }
        let num_y = top + parts_h as f32;
        size.draw(p, &total, num_y, style(colour, alpha));
        let label_y = num_y + size.height() as f32 + 4.0;
        match tag {
            Some((t, tc)) => {
                // The tag sits on the label's baseline.
                let w = TAG.measure(t) + 4 + TEXT.measure(&label);
                let x0 = -floorf(w as f32 / 2.0);
                TAG.draw(p, t, x0, label_y + 2.0, 1.0, Align::Left, style(tc, alpha));
                let lx = x0 + (TAG.measure(t) + 4) as f32;
                TEXT.draw(p, &label, lx, label_y, 1.0, Align::Left, style(pal::DIM, alpha));
            }
            None => TEXT.draw(
                p,
                &label,
                0.0,
                label_y,
                1.0,
                Align::Center,
                style(pal::DIM, alpha),
            ),
        }
    });
}

// ---------- idle: the wake label (C2) ----------

/// The number a face would carry on an ordinary die: the boot's pips, so
/// opposite faces sum to 7.
pub fn face_number(face: usize) -> u8 {
    FACE_START[face % 6]
}

/// The die at rest: its solid in violet, the setup under it, and which face
/// this is in a small tag. Games keep the 96×96 label for now.
pub fn draw_idle<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    setup: Setup,
    label: &str,
    alpha: f32,
    face: usize,
) {
    let die = match setup {
        Setup::Roll(die, _) if die.is_numeric() => die,
        _ => {
            screens::draw_wake_label(c, setup, label, alpha);
            return;
        }
    };
    in_pixels(c, |p| {
        draw_centered(p, die_icon(die), 0.0, -10.0, 1.0, style(pal::VIOLET, alpha));
        let big = TEXT.measure(label) * 2 <= LINE_MAX;
        let scale = if big { 2.0 } else { 1.0 };
        TEXT.draw(p, label, 0.0, 4.0, scale, Align::Center, style(pal::WHITE, alpha));
        let mut tag: String<8> = String::new();
        let _ = write!(tag, "FACE {}", face_number(face));
        TAG.draw(p, &tag, 0.0, 22.0, 1.0, Align::Center, style(pal::DIM, alpha));
    });
}

// ---------- boot (C1) ----------

/// Pip slots for a value, in units of the pip grid (as the 96×96 boot).
fn pip_slots(value: u8) -> &'static [(i8, i8)] {
    match value {
        1 => &[(0, 0)],
        2 => &[(-1, -1), (1, 1)],
        3 => &[(-1, -1), (1, 1), (0, 0)],
        4 => &[(-1, -1), (1, 1), (1, -1), (-1, 1)],
        5 => &[(-1, -1), (1, 1), (1, -1), (-1, 1), (0, 0)],
        _ => &[(-1, -1), (1, 1), (1, -1), (-1, 1), (-1, 0), (1, 0)],
    }
}

/// Pip grid spacing, px.
const PIP_STEP: f32 = 14.0;

fn draw_pips<T: Target>(p: &mut Painter<T>, value: u8, colour: Color, alpha: f32) {
    for &(sx, sy) in pip_slots(value) {
        draw_centered(
            p,
            &spr::PIP,
            sx as f32 * PIP_STEP,
            sy as f32 * PIP_STEP,
            1.0,
            style(colour, alpha),
        );
    }
}

/// The boot on one face: the pips count round as on the 96×96 die (same
/// timing), then the top face shows the sugar cube and writes `sugarcube`
/// under it, with a gold sparkle.
pub fn draw_boot<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, index: usize, top: bool, t: f32) {
    let t = t - if top { 0.0 } else { index as f32 * 0.05 };
    if t < 0.0 {
        return;
    }
    if top && t >= LOOP_END {
        in_pixels(c, |p| draw_boot_finale(p, t - LOOP_END));
        return;
    }
    let v0 = if top { 6 } else { FACE_START[index] };
    let val = |k: u32| ((v0 as u32 - 1 + k) % 6 + 1) as u8;
    let appear = (t / 0.25).min(1.0);
    let steps_t = t - 0.25;
    let fade = ((steps_t - BOOT_STEP * BOOT_STEPS as f32) / BOOT_FADE).clamp(0.0, 1.0);
    // Pixel pips can't slide by fractions: each step lands at once, half
    // way through the 96×96 die's tween.
    let value = if steps_t < 0.0 {
        v0
    } else {
        let k = floorf(steps_t / BOOT_STEP) as u32;
        let into = steps_t - k as f32 * BOOT_STEP;
        val((k + (into >= 0.11) as u32).min(BOOT_STEPS))
    };
    in_pixels(c, |p| draw_pips(p, value, pal::WHITE, appear * (1.0 - fade)));
}

fn draw_boot_finale<T: Target>(p: &mut Painter<T>, u: f32) {
    let fade = 1.0 - ((u - 3.8) / 0.4).clamp(0.0, 1.0);
    if fade <= 0.0 {
        return;
    }
    // The last pips go as the cube arrives.
    if u < 0.35 {
        draw_pips(p, 5, pal::WHITE, 1.0 - u / 0.35);
    }
    let cube = (u / 0.3).min(1.0) * fade;
    // The 15×15 cube at 2×, its layers placed from the shared box (each
    // layer alone is shorter than the cube).
    let (x, y) = (-15.0, -27.0);
    draw_glyph(p, &spr::CUBE_TOP, x, y, 2.0, style(pal::WHITE, cube));
    draw_glyph(p, &spr::CUBE_LEFT, x, y, 2.0, style(Color::hex(0xC9CCD3), cube));
    draw_glyph(p, &spr::CUBE_RIGHT, x, y, 2.0, style(pal::DIM, cube));
    // `sugarcube` writes itself on, a letter at a time.
    const WORD: &str = "sugarcube";
    let w = (((u - 1.15) / 0.95).clamp(0.0, 1.0) * WORD.len() as f32 + 0.999) as usize;
    if u >= 1.15 && w > 0 {
        let x0 = -floorf(TEXT.measure(WORD) as f32 / 2.0);
        TEXT.draw(
            p,
            &WORD[..w.min(WORD.len())],
            x0,
            10.0,
            1.0,
            Align::Left,
            style(pal::WHITE, fade),
        );
    }
    // A sparkle on the cube's corner.
    let s = (u - 2.25) / 0.5;
    if (0.0..1.0).contains(&s) {
        let a = sinf(core::f32::consts::PI * s);
        draw_centered(p, &spr::SPARKLE, 15.0, -24.0, 1.0, style(pal::GOLD, a * fade));
    }
}

// ---------- menu (C3): the die picker and the other pages ----------

/// One menu page: status bar (setup and battery), title, ▲, the value, ▼
/// and page dots. On the Which die page the value is the die.
#[allow(clippy::too_many_arguments)]
pub fn draw_menu<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    m: &Draft,
    battery: f32,
    ox: f32,
    oy: f32,
    alpha: f32,
    scale: f32,
) {
    if alpha <= 0.0 {
        return;
    }
    let base = c.painter.xf;
    c.painter.xf = base.offset(ox, oy).scaled(scale).in_pixels();
    let p = &mut *c.painter;

    // Status bar.
    let short = m.setup().short_label();
    TAG.draw(p, &short, -22.0, -28.0, 1.0, Align::Left, style(pal::DIM, alpha));
    draw_glyph(p, &spr::BATTERY, 11.0, -28.0, 1.0, style(pal::DIM, alpha));
    let level = battery.clamp(0.0, 1.0);
    let fill = if level <= 0.15 { pal::RED } else { pal::MINT };
    if level > 0.0 {
        p.fill_rect(13.0, -26.0, roundf(5.0 * level).max(1.0), 1.0, style(fill, alpha));
    }

    let mut title: String<12> = String::new();
    let _ = title.push_str(page_title(m.page));
    if let Page::Token(i) = m.page {
        title.clear();
        let _ = write!(title, "Player {}", i + 1);
    }
    TEXT.draw(p, &title, 0.0, -20.0, 1.0, Align::Center, style(pal::DIM, alpha));
    // A Settings value that takes more than a line uses ▼'s room.
    let setting = (m.page == Page::Settings).then(|| SettingRows::new(m));
    if m.page != Page::EndGame {
        draw_centered(p, &spr::UP, 0.0, -9.0, 1.0, style(pal::VIOLET, alpha));
        if setting.as_ref().is_none_or(|s| s.compact()) {
            draw_centered(p, &spr::DOWN, 0.0, 17.0, 1.0, style(pal::VIOLET, alpha));
        }
    }

    match m.page {
        Page::EndGame => {
            // "Scores won't be kept" needs three lines here and would run
            // into the page dots; this says the same in two.
            draw_lines(p, "Hold to end", 0.0, -7.0, pal::WHITE, alpha);
            draw_lines(p, "Scores are lost", 0.0, 4.0, pal::DIM, alpha);
        }
        Page::Settings => {
            if let Some(rows) = &setting {
                rows.draw(p, alpha);
            }
        }
        _ => {
            let value = m.value();
            let colour = if m.page == Page::Die {
                pal::VIOLET
            } else {
                pal::WHITE
            };
            if TEXT.measure(&value) * 2 <= LINE_MAX {
                TEXT.draw(p, &value, 0.0, -3.0, 2.0, Align::Center, style(colour, alpha));
            } else {
                let mut lines: Vec<&str, 2> = Vec::new();
                wrap(&TEXT, &value, LINE_MAX, &mut lines);
                let y0 = if lines.len() > 1 { -5.0 } else { 0.0 };
                for (i, l) in lines.iter().enumerate() {
                    TEXT.draw(
                        p,
                        l,
                        0.0,
                        y0 + i as f32 * LINE_PITCH,
                        1.0,
                        Align::Center,
                        style(colour, alpha),
                    );
                }
            }
        }
    }

    // Page dots.
    let n = m.ring().len();
    for i in 0..n {
        let x = (i as f32 - (n as f32 - 1.0) / 2.0) * 5.0;
        let x = floorf(x);
        let on = i == m.page_index();
        p.fill_rect(
            x,
            24.0,
            2.0,
            2.0,
            style(if on { pal::WHITE } else { pal::FAINT }, alpha),
        );
    }
    c.painter.xf = base;
}

/// A Settings item laid out: its name, its value in 5×7 (wrapped to two
/// lines at most) and any detail in 3×5 tags.
pub struct SettingRows {
    pub name: &'static str,
    value: String<24>,
    detail: Option<&'static str>,
}

impl SettingRows {
    pub fn new(m: &Draft) -> Self {
        // The one name too long for a 64×64 line.
        let name = match m.setting().0 {
            "Sleep after" => "Sleep",
            n => n,
        };
        Self {
            name,
            value: m.setting_value(),
            detail: m.setting_detail(),
        }
    }

    /// One line of value and no detail: ▼ keeps its place.
    pub fn compact(&self) -> bool {
        self.detail.is_none() && TEXT.measure(&self.value) <= LINE_MAX
    }

    /// Each row's text, top, and whether it's a 3×5 tag. `None` if it
    /// doesn't fit the page.
    pub fn rows(&self) -> Option<Vec<(&str, f32, bool), 6>> {
        let mut out = Vec::new();
        let name_y = if self.compact() { -5.0 } else { -7.0 };
        out.push((self.name, name_y, false)).ok()?;
        let mut values: Vec<&str, 2> = Vec::new();
        if !wrap(&TEXT, &self.value, LINE_MAX, &mut values) {
            return None;
        }
        let mut y = name_y + LINE_PITCH;
        for v in values {
            out.push((v, y, false)).ok()?;
            y += LINE_PITCH;
        }
        if let Some(d) = self.detail {
            let mut tags: Vec<&str, 2> = Vec::new();
            if !wrap(&TAG, d, LINE_MAX, &mut tags) {
                return None;
            }
            for t in tags {
                out.push((t, y, true)).ok()?;
                y += 6.0;
            }
        }
        Some(out)
    }

    /// Where the last row ends: the page dots start at 24.
    pub fn bottom(&self) -> Option<f32> {
        let rows = self.rows()?;
        rows.last().map(|&(_, y, tag)| y + if tag { 5.0 } else { 7.0 })
    }

    fn draw<T: Target>(&self, p: &mut Painter<T>, alpha: f32) {
        for (i, (text, y, tag)) in self.rows().unwrap_or_default().into_iter().enumerate() {
            let colour = if i == 0 { pal::WHITE } else { pal::DIM };
            let font_style = style(colour, alpha);
            if tag {
                TAG.draw(p, text, 0.0, y, 1.0, Align::Center, font_style);
            } else {
                TEXT.draw(p, text, 0.0, y, 1.0, Align::Center, font_style);
            }
        }
    }
}

/// Menu titles that fit a 64×64 line (the 96×96 die's "How many dice" and
/// "Bills in hand" don't).
pub fn page_title(page: Page) -> &'static str {
    match page {
        Page::Mode => "Mode",
        Page::Count => "How many",
        Page::Die => "Which die",
        Page::Pot => "Bills",
        Page::Fuse => "Fuse",
        Page::Players => "Players",
        Page::Token(_) => "Player",
        Page::EndGame => "End game",
        Page::Settings => "Settings",
    }
}

// ---------- saved (C4) ----------

/// After a save: a mint check that draws itself, the setup, and a small
/// `READY TO ROLL`.
pub fn draw_success<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, label: &str, nudge: &str, t: f32) {
    let a = if t < 0.15 {
        t / 0.15
    } else {
        1.0 - ((t - 0.9) / 0.35).clamp(0.0, 1.0)
    };
    if a <= 0.0 {
        return;
    }
    // The check draws left to right from 0.05 s to 0.35 s.
    let cols = (((t - 0.05) / 0.3).clamp(0.0, 1.0) * spr::CHECK.width as f32 + 0.999) as u32;
    let mask = if cols >= 32 { u32::MAX } else { !(u32::MAX >> cols) };
    let mut check = spr::CHECK;
    for r in check.rows.iter_mut() {
        *r &= mask;
    }
    in_pixels(c, |p| {
        draw_centered(p, &check, 0.0, -14.0, 1.0, style(pal::MINT, a));
        let scale = if TEXT.measure(label) * 2 <= LINE_MAX {
            2.0
        } else {
            1.0
        };
        TEXT.draw(p, label, 0.0, -3.0, scale, Align::Center, style(pal::WHITE, a));
        let mut upper: String<24> = String::new();
        for ch in nudge.chars() {
            let _ = upper.push(ch.to_ascii_uppercase());
        }
        if TAG.covers(&upper) && TAG.measure(&upper) <= LINE_MAX {
            TAG.draw(p, &upper, 0.0, 17.0, 1.0, Align::Center, style(pal::DIM, a));
        } else {
            draw_lines(p, nudge, 0.0, 16.0, pal::DIM, a);
        }
    });
}

// ---------- charging and low battery (C7, C7a) ----------

/// The Nest's top face (the charge) in 64×64; `false` for the screens that
/// keep their 96×96 layout (the clock, the guidance).
pub fn draw_nest<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, face: &NestFace) -> bool {
    let Screen::Charge(v) = face.screen else {
        return false;
    };
    let base = c.painter.xf;
    // The burn-in shift, in whole pixels.
    let k = crate::gfx::k_of::<T>();
    let (sx, sy) = (roundf(face.shift.0 * k) / k, roundf(face.shift.1 * k) / k);
    c.painter.xf = base.offset(sx, sy).in_pixels();
    draw_charge(c.painter, &v);
    c.painter.xf = base;
    true
}

fn draw_charge<T: Target>(p: &mut Painter<T>, v: &ChargeView) {
    if v.alpha <= 0.0 {
        return;
    }
    let low = v.pct <= crate::LOW_BATTERY_PCT as f32 && v.label == Label::Battery;
    let tint = if low { pal::RED } else { pal::MINT };
    // The fill rises from the bottom with a gentle wave.
    let surface = 32.0 - 64.0 * v.fill.clamp(0.0, 1.0);
    let wave = v.wave;
    let fill_alpha = (0.28 + 0.5 * v.flash) * v.alpha;
    if v.fill > 0.0 {
        p.shape(
            (-32.0, surface - 2.0, 32.0, 32.0),
            style(tint, fill_alpha),
            |x, y| surface + 1.2 * sinf(0.3 * x + 2.2 * wave) - y,
        );
    }
    let text = v.alpha * v.text;
    let mut pct: String<4> = String::new();
    let _ = write!(pct, "{}", roundf(v.pct.clamp(0.0, 100.0)) as u16);
    let wn = NUM_S.measure(&pct);
    let wp = TEXT.measure("%");
    let total = wn + 2 + wp;
    let x0 = -floorf(total as f32 / 2.0);
    NUM_S.draw(p, &pct, x0 + (wn / 2) as f32, -15.0, style(pal::WHITE, text));
    TEXT.draw(
        p,
        "%",
        x0 + (wn + 2) as f32,
        0.0,
        1.0,
        Align::Left,
        style(pal::WHITE, text),
    );
    let word = match v.label {
        Label::Charging => "charging",
        Label::Full => "full",
        Label::Battery => "battery",
    };
    TEXT.draw(
        p,
        word,
        0.0,
        12.0,
        1.0,
        Align::Center,
        style(if low { pal::RED } else { pal::DIM }, text),
    );
}

/// The charging face's low-battery mark: the bolt, and `low` in red.
pub fn draw_low_battery<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
    in_pixels(c, |p| {
        draw_centered(p, &spr::BOLT, 0.0, -6.0, 1.0, style(pal::EMBER, alpha));
        TEXT.draw(p, "low", 0.0, 9.0, 1.0, Align::Center, style(pal::RED, alpha));
    });
}

// ---------- text: how much fits ----------

/// A roll with a modifier, `1d20 + 5 = 17`: the setup and modifier on top
/// (the modifier mint, or ember if negative), the total big, and the roll
/// itself under it. **Not reachable on the die yet**: the firmware has no
/// modifiers. Drawn for the contact sheet, to judge how much text a face
/// holds.
pub fn draw_modifier<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    setup: &str,
    rolled: u16,
    modifier: i16,
    alpha: f32,
) {
    let total = (rolled as i32 + modifier as i32).max(0);
    let (sign, tint) = if modifier < 0 {
        ("-", pal::EMBER)
    } else {
        ("+", pal::MINT)
    };
    let mut m: String<8> = String::new();
    let _ = write!(m, "{sign} {}", modifier.unsigned_abs());
    let mut r: String<8> = String::new();
    let _ = write!(r, "{rolled}");
    let mut t: String<8> = String::new();
    let _ = write!(t, "{total}");
    let size = num_size(t.len(), true);
    in_pixels(c, |p| {
        draw_runs(
            p,
            &TEXT,
            &[(setup, pal::DIM), (" ", pal::DIM), (&m, tint)],
            0.0,
            -27.0,
            1.0,
            alpha,
        );
        size.draw(p, &t, -16.0, style(pal::WHITE, alpha));
        let below = -16.0 + size.height() as f32 + 4.0;
        draw_runs(
            p,
            &TEXT,
            &[(&r, pal::WHITE), (" ", pal::DIM), (&m, tint)],
            0.0,
            below,
            1.0,
            alpha,
        );
    });
}

/// Lines of 5×7 text, wrapped to the panel and centred: the multi-line
/// screen for judging how much text fits. Up to six lines of about ten
/// characters. Returns whether it all fit.
pub fn draw_text_block<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    text: &str,
    colour: Color,
    alpha: f32,
) -> bool {
    let mut lines: Vec<&str, 6> = Vec::new();
    let fits = wrap(&TEXT, text, LINE_MAX, &mut lines);
    let h = lines.len() as f32 * LINE_PITCH - 2.0;
    let y0 = -floorf(h / 2.0);
    in_pixels(c, |p| {
        for (i, l) in lines.iter().enumerate() {
            TEXT.draw(
                p,
                l,
                0.0,
                y0 + i as f32 * LINE_PITCH,
                1.0,
                Align::Center,
                style(colour, alpha),
            );
        }
    });
    fits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_spaces_within_the_line() {
        let mut lines: Vec<&str, 6> = Vec::new();
        assert!(wrap(&TEXT, "Scores won't be kept", LINE_MAX, &mut lines));
        assert_eq!(lines.as_slice(), &["Scores", "won't be", "kept"]);
        assert!(wrap(&TEXT, "Scores are lost", LINE_MAX, &mut lines));
        assert_eq!(lines.as_slice(), &["Scores are", "lost"]);
        assert!(lines.iter().all(|l| TEXT.measure(l) <= LINE_MAX));
        assert!(wrap(&TEXT, "d20", LINE_MAX, &mut lines));
        assert_eq!(lines.as_slice(), &["d20"]);
        // Six lines and no more.
        let long = "the quick brown fox jumps over the lazy dog and keeps running far away";
        assert!(!wrap(&TEXT, long, LINE_MAX, &mut lines));
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn a_word_wider_than_a_line_is_cut() {
        let mut lines: Vec<&str, 6> = Vec::new();
        assert!(wrap(&TEXT, "abcdefghijklmnopqrstuvwxyz", LINE_MAX, &mut lines));
        assert!(lines.len() >= 3);
        assert!(lines.iter().all(|l| TEXT.measure(l) <= LINE_MAX));
    }

    #[test]
    fn every_setting_fits_its_page() {
        use crate::menu::{Settings, SETTINGS};
        let mut d = Draft::new(&Settings::default());
        d.page = Page::Settings;
        for (i, item) in SETTINGS.iter().enumerate() {
            d.setting = i as u8;
            for k in 0..item.options.len() {
                d.choices[i] = k as u8;
                let rows = SettingRows::new(&d);
                assert!(TEXT.measure(rows.name) <= LINE_MAX, "{}", rows.name);
                let bottom = rows
                    .bottom()
                    .unwrap_or_else(|| panic!("{} doesn't fit", rows.name));
                // The page dots start at 24: keep a dark row above them.
                assert!(bottom <= 22.0, "{}: ends at {bottom}", rows.name);
            }
        }
    }

    #[test]
    fn every_menu_value_fits() {
        use crate::menu::{PlayMode, Settings};
        let mut d = Draft::new(&Settings::default());
        for play in PlayMode::ALL {
            d.page = Page::Mode;
            d.play = play;
            let v = d.value();
            let mut lines: Vec<&str, 2> = Vec::new();
            assert!(wrap(&TEXT, &v, LINE_MAX, &mut lines), "{v}");
        }
        for die in DieKind::NUMERIC {
            d.page = Page::Die;
            d.die = die;
            assert!(TEXT.measure(&d.value()) * 2 <= LINE_MAX, "{die:?} at 2×");
        }
    }

    #[test]
    fn every_result_fits() {
        use smokebomb_shared::types::MAX_DICE;
        for die in DieKind::NUMERIC {
            for n in 1..=MAX_DICE as u8 {
                // The widest total and the longest label this setup can make.
                let label = setup_label(die, n);
                let max = die.sides() as u16 * n as u16;
                let mut t: String<8> = String::new();
                let _ = write!(t, "{max}");
                let size = num_size(t.len(), n > 1);
                assert!(size.measure(&t) <= LINE_MAX + 2, "{n}{die:?}: total {t}");
                let tagged = TAG.measure("MAX") + 4 + TEXT.measure(&label);
                assert!(tagged <= LINE_MAX, "{n}{die:?}: {label}");
                // Parts, if shown, fit their line.
                let values = [die.sides(); MAX_DICE];
                if let Some((p, tag)) = parts_line(&values[..n as usize]) {
                    let w = if tag { TAG.measure(&p) } else { TEXT.measure(&p) };
                    assert!(w <= LINE_MAX, "{p}");
                }
                // The idle label at its size.
                let w = TEXT.measure(&label);
                assert!(w <= LINE_MAX, "{label}");
            }
        }
    }

    #[test]
    fn menu_titles_fit_a_line() {
        for page in [
            Page::Mode,
            Page::Count,
            Page::Die,
            Page::Pot,
            Page::Fuse,
            Page::Players,
            Page::Token(5),
            Page::EndGame,
            Page::Settings,
        ] {
            assert!(TEXT.measure(page_title(page)) <= LINE_MAX, "{page:?}");
        }
    }

    #[test]
    fn sizes_by_digits() {
        assert_eq!(num_size(1, false), NumSize::Large);
        assert_eq!(num_size(2, false), NumSize::Large);
        assert_eq!(num_size(2, true), NumSize::Medium);
        assert_eq!(num_size(3, false), NumSize::Medium);
        assert_eq!(num_size(4, false), NumSize::Small);
    }

    #[test]
    fn parts_fall_back_to_tags_then_nothing() {
        assert_eq!(parts_line(&[7]), None);
        let (s, tag) = parts_line(&[3, 5, 2]).unwrap();
        assert_eq!((s.as_str(), tag), ("3+5+2", false));
        let (_, tag) = parts_line(&[20, 20, 20, 20, 20]).unwrap();
        assert!(tag, "five d20s need the small tags");
        assert_eq!(parts_line(&[20; 10]), None);
    }

    #[test]
    fn face_numbers_are_a_die() {
        // Opposite faces (+X/−X, +Y/−Y, +Z/−Z) sum to 7.
        for f in [0, 2, 4] {
            assert_eq!(face_number(f) + face_number(f + 1), 7);
        }
    }
}
