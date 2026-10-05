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
use smokebomb_shared::{DieKind, PotFace, RollRecord};

use crate::font64::{draw_centered, draw_glyph, Align, BitFont, Glyph, NUM_L, NUM_M, NUM_S, TEXT};
use crate::gfx::{Painter, Style};
use crate::menu::{AppIcon, Draft, Page, PlayMode, Setup, Value};
use crate::nest::{ChargeView, Label, NestFace, Screen};
use crate::palette64 as pal;
use crate::pigs::{Locked, Outcome, Symbol, Throw, Token};
use crate::screens::{self, Ctx, BOOT_FADE, BOOT_STEP, BOOT_STEPS, FACE_START, LOOP_END};
use crate::smoke::Special;
use crate::sprites64 as spr;
use crate::tier64::{H1, T1, T2};

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

// ---------- table screens (brief 3, 1.2) ----------
//
// Read from across the table: T1, the answer (30 px numerals, or 21 px
// letters, or a glyph at least 21 px tall), in a colour that means the same
// in every game, over at most one T2 word (15 px caps). Nothing smaller, and
// no hints: a tap shows those.

/// T2's cap height, and the gap between T1 and T2.
const T2_CAP: f32 = 15.0;
const T1_GAP: f32 = 5.0;
/// T1 in the result numerals, and in letters.
const T1_NUM: f32 = 30.0;
const T1_TEXT: f32 = 21.0;

/// The tops of T1 (`h1` tall) and T2, centred together on the face.
fn table_layout(h1: f32, t2: bool) -> (f32, f32) {
    let block = if t2 { h1 + T1_GAP + T2_CAP } else { h1 };
    let top = -floorf(block / 2.0);
    (top, top + h1 + T1_GAP)
}

/// One T2 word, centred, its cap top at `y`.
fn draw_t2<T: Target>(p: &mut Painter<T>, text: &str, y: f32, colour: Color, alpha: f32) {
    T2.draw(p, text, 0.0, y, 1.0, Align::Center, style(colour, alpha));
}

/// A T1 answer of up to three characters, `T1_NUM` tall from `y`: the
/// result numerals for a plain number, a sign drawn to match before them,
/// or 21 px letters (centred in the same height) for anything else.
fn draw_t1<T: Target>(p: &mut Painter<T>, text: &str, y: f32, colour: Color, alpha: f32) {
    let s = style(colour, alpha);
    let (sign, digits) = match text.chars().next() {
        Some(c @ ('+' | '−')) => (Some(c), &text[c.len_utf8()..]),
        _ => (None, text),
    };
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        let (arm, stem, gap) = (14.0, 4.0, 3.0);
        let wd = NUM_M.measure(digits) as f32;
        let w = wd + if sign.is_some() { arm + gap } else { 0.0 };
        let x0 = -floorf(w / 2.0);
        if let Some(sign) = sign {
            let mid = y + 15.0;
            p.fill_rect(x0, mid - stem / 2.0, arm, stem, s);
            if sign == '+' {
                p.fill_rect(x0 + (arm - stem) / 2.0, mid - arm / 2.0, stem, arm, s);
            }
        }
        let dx = x0 + if sign.is_some() { arm + gap } else { 0.0 };
        NUM_M.draw(p, digits, dx + floorf(wd / 2.0), y, s);
    } else {
        T1.draw(p, text, 0.0, y + (T1_NUM - T1_TEXT) / 2.0, 1.0, Align::Center, s);
    }
}

/// A player's token as T1, 21 px, centred, its top at `y`: an initial in
/// the T1 letters, or a symbol at 3×.
fn draw_t1_token<T: Target>(p: &mut Painter<T>, token: Token, y: f32, colour: Color, alpha: f32) {
    match (token.initial(), token.as_symbol()) {
        (_, Some(sym)) => {
            draw_centered(p, symbol_sprite(sym), 0.0, y + 10.0, 3.0, style(colour, alpha));
            p.note_icon(21.0, alpha);
        }
        (Some(ch), _) => {
            let mut s: String<2> = String::new();
            let _ = s.push(ch);
            T1.draw(p, &s, 0.0, y, 1.0, Align::Center, style(colour, alpha));
        }
        _ => {}
    }
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

/// A roll's result: the total as T1 (white, gold for a max, red for a dud)
/// over the dice, or `MAX` / `DUD`, as T2. Pass the Pot shows its dice as
/// glyphs ([`draw_pot_result`]).
pub fn draw_result<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    record: &RollRecord,
    special: Option<Special>,
    alpha: f32,
) {
    if !record.die.is_numeric() {
        draw_pot_result(c, record, special == Some(Special::Dud), alpha);
        return;
    }
    let total = crate::table::total(record.total());
    let dice = crate::table::dice(record.die, record.values.len() as u8);
    let (colour, word, word_colour) = match special {
        Some(Special::Max) => (pal::GOLD, "MAX", pal::GOLD),
        Some(Special::Dud) => (pal::RED, "DUD", pal::RED),
        None => (pal::WHITE, dice.as_str(), pal::DIM),
    };
    let (y1, y2) = table_layout(T1_NUM, true);
    in_pixels(c, |p| {
        draw_t1(p, &total, y1, colour, alpha);
        draw_t2(p, word, y2, word_colour, alpha);
    });
}

// ---------- idle: the wake label (C2) ----------

/// The number a face would carry on an ordinary die: the boot's pips, so
/// opposite faces sum to 7.
pub fn face_number(face: usize) -> u8 {
    FACE_START[face % 6]
}

/// The die at rest (a table screen): its solid in violet at 2× as T1, the
/// dice as T2. Pass the Pot shows the bills in hand, and Hot Potato its
/// potato; Pig Toss has its own label ([`draw_pigs_label`]).
pub fn draw_idle<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    setup: Setup,
    label: &str,
    alpha: f32,
    face: usize,
) {
    let _ = (label, face);
    let (die, count) = match setup {
        Setup::Roll(die, n) if die.is_numeric() => (die, n),
        Setup::Roll(DieKind::PassThePot, n) => {
            draw_bills(c, n, alpha);
            return;
        }
        Setup::HotPotato => {
            draw_potato_label(c, alpha);
            return;
        }
        _ => {
            screens::draw_wake_label(c, setup, label, alpha);
            return;
        }
    };
    let icon = die_icon(die);
    let h = 2.0 * icon.rows.iter().filter(|&&r| r != 0).count() as f32;
    let (y1, y2) = table_layout(h, true);
    in_pixels(c, |p| {
        draw_centered(p, icon, 0.0, y1 + floorf(h / 2.0), 2.0, style(pal::VIOLET, alpha));
        p.note_icon(h, alpha);
        draw_t2(p, &crate::table::dice(die, count), y2, pal::WHITE, alpha);
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

/// One menu page (a held screen, brief 3, 1.3): the setup and the title in
/// H2 at the top, the value in H1 between ▲ and ▼ (or a caption under it),
/// and the page dots. Battery has a page of its own in Settings.
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
    let view = m.view((battery * 100.0 + 0.5) as u8);
    let base = c.painter.xf;
    c.painter.xf = base.offset(ox, oy).scaled(scale).in_pixels();
    let p = &mut *c.painter;

    TEXT.draw(
        p,
        &view.status,
        0.0,
        -31.0,
        1.0,
        Align::Center,
        style(pal::FAINT, alpha),
    );
    TEXT.draw(
        p,
        &view.title,
        0.0,
        -22.0,
        1.0,
        Align::Center,
        style(pal::DIM, alpha),
    );
    if view.arrows {
        draw_centered(p, &spr::UP, 0.0, -11.0, 1.0, style(pal::VIOLET, alpha));
        if view.caption.is_none() {
            draw_centered(p, &spr::DOWN, 0.0, 16.0, 1.0, style(pal::VIOLET, alpha));
        }
    }
    // The value's box: H1 from y -6 to 11, or with a caption, from -7.
    let top = if view.caption.is_some() { -7.0 } else { -6.0 };
    match &view.value {
        Value::Text(t) => {
            let colour = if m.page == Page::Die {
                pal::VIOLET
            } else {
                pal::WHITE
            };
            draw_value(p, t, top, style(colour, alpha));
        }
        Value::Token(t) => match (t.initial(), t.as_symbol()) {
            (_, Some(sym)) => {
                draw_centered(
                    p,
                    symbol_sprite(sym),
                    0.0,
                    top + 9.0,
                    3.0,
                    style(pal::PINK, alpha),
                );
                p.note_icon(21.0, alpha);
            }
            (Some(ch), _) => {
                let mut s: String<2> = String::new();
                let _ = s.push(ch);
                draw_value(p, &s, top, style(pal::PINK, alpha));
            }
            _ => {}
        },
        Value::App(icon) => draw_app_icon(p, *icon, m.die, top + 6.0, alpha),
        Value::Lines([a, b]) => {
            TEXT.draw(p, a, 0.0, -6.0, 1.0, Align::Center, style(pal::DIM, alpha));
            TEXT.draw(p, b, 0.0, 3.0, 1.0, Align::Center, style(pal::WHITE, alpha));
        }
    }
    if let Some(caption) = view.caption {
        TEXT.draw(p, caption, 0.0, 13.0, 1.0, Align::Center, style(pal::DIM, alpha));
    }

    // Page dots, 3 px so they read as dots.
    for i in 0..view.dots {
        let x = floorf((i as f32 - (view.dots as f32 - 1.0) / 2.0) * 6.0) - 1.0;
        let on = i == view.dot;
        p.fill_rect(
            x,
            24.0,
            3.0,
            3.0,
            style(if on { pal::WHITE } else { pal::FAINT }, alpha),
        );
    }
    c.painter.xf = base;
}

/// A held page's value in H1, centred with its top at `y`.
fn draw_value<T: Target>(p: &mut Painter<T>, text: &str, y: f32, s: Style) {
    H1.draw(p, text, 0.0, y, 1.0, Align::Center, s);
}

/// An app's picture on the Apps page, centred on `cy`: the die in use, a
/// bill, the potato, the pig, or a gear.
fn draw_app_icon<T: Target>(p: &mut Painter<T>, icon: AppIcon, die: DieKind, cy: f32, alpha: f32) {
    match icon {
        AppIcon::Game(PlayMode::Dice) => {
            draw_centered(p, die_icon(die), 0.0, cy, 1.0, style(pal::VIOLET, alpha))
        }
        AppIcon::Game(PlayMode::PassThePot) => {
            draw_centered(p, &spr::BILL, 0.0, cy, 2.0, style(pal::MINT, alpha))
        }
        AppIcon::Game(PlayMode::HotPotato) => {
            let x = -floorf(spr::POTATO_SKIN.width as f32 / 2.0);
            draw_glyph(p, &spr::POTATO_SKIN, x, cy - 6.0, 1.0, style(pal::POTATO, alpha));
            draw_glyph(
                p,
                &spr::POTATO_SPOTS,
                x,
                cy - 6.0,
                1.0,
                style(pal::POTATO_DARK, alpha),
            );
        }
        AppIcon::Game(PlayMode::PigToss) => draw_pig_face(p, -9.0, cy - 8.0, 1.0, alpha),
        AppIcon::Settings => draw_centered(p, &spr::GEAR, 0.0, cy, 1.0, style(pal::WHITE, alpha)),
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
        draw_centered(p, &check, 0.0, -17.0, 1.0, style(pal::MINT, a));
        // The setup as H1 if it fits a line, else H2; how to start under it.
        if H1.measure(label) <= LINE_MAX {
            H1.draw(p, label, 0.0, -7.0, 1.0, Align::Center, style(pal::WHITE, a));
        } else {
            TEXT.draw(p, label, 0.0, -2.0, 1.0, Align::Center, style(pal::WHITE, a));
        }
        TEXT.draw(p, nudge, 0.0, 17.0, 1.0, Align::Center, style(pal::DIM, a));
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

/// The charging face's low-battery mark: the bolt at 2× as T1, and `low`
/// in red as T2.
pub fn draw_low_battery<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
    let h = 28.0;
    let (y1, y2) = table_layout(h, true);
    in_pixels(c, |p| {
        draw_centered(p, &spr::BOLT, 0.0, y1 + h / 2.0, 2.0, style(pal::EMBER, alpha));
        p.note_icon(h, alpha);
        draw_t2(p, "low", y2, pal::RED, alpha);
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

// ---------- Pig Toss ----------
//
// The 96×96 die fits a lot on a face at once: the shrunken pigs, the
// throw's points, the pose, the turn's tally and two lines of prompts. At
// 64×64 that is unreadable, so these screens take turns instead: the pigs
// settle at full size and fade (the target's `pig_settle`), the throw's
// points pop in with the pose under them, then a resting screen shows only
// whose turn it is, the turn's points, and two hints as icons: shake to roll
// again, or bank (a padlock and what the score would be). Players' tokens
// are pink, and so are the pigs.

/// A player's symbol as a 9×7 sprite.
pub fn symbol_sprite(symbol: Symbol) -> &'static Glyph<7> {
    match symbol {
        Symbol::Hat => &spr::HAT,
        Symbol::Car => &spr::CAR,
        Symbol::Boot => &spr::BOOT,
        Symbol::Boat => &spr::BOAT,
        Symbol::Crown => &spr::CROWN,
        Symbol::Star => &spr::STAR,
    }
}

/// A token's width in pixels at scale 1 (a capital is 5, a symbol 9).
pub fn token_width(token: Token) -> usize {
    match token.as_symbol() {
        Some(_) => 9,
        None => token
            .initial()
            .and_then(|c| TEXT.find(c))
            .map_or(5, |g| g.width as usize),
    }
}

/// The happy pig, 19×16 at `scale`, its top-left at `(x, y)`.
fn draw_pig_face<T: Target>(p: &mut Painter<T>, x: f32, y: f32, scale: f32, alpha: f32) {
    draw_glyph(p, &spr::PIG_FACE, x, y, scale, style(pal::PINK, alpha));
    draw_glyph(p, &spr::PIG_SNOUT, x, y, scale, style(pal::PINK_LIGHT, alpha));
    draw_glyph(p, &spr::PIG_DARK, x, y, scale, style(pal::PINK_DARK, alpha));
}

/// 0 before `at`, rising to 1 over `over` seconds.
fn ramp(t: f32, at: f32, over: f32) -> f32 {
    ((t - at) / over).clamp(0.0, 1.0)
}

/// Between turns (a table screen): whose go it is as T1, their token in
/// pink, and `turn` as T2; once someone has won, their token in gold and
/// `wins`.
pub fn draw_pigs_label<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, token: Token, won: bool, alpha: f32) {
    let (y1, y2) = table_layout(T1_TEXT, true);
    in_pixels(c, |p| {
        let (colour, word) = if won {
            (pal::GOLD, "wins")
        } else {
            (pal::PINK, "turn")
        };
        draw_t1_token(p, token, y1, colour, alpha);
        draw_t2(p, word, y2, pal::WHITE, alpha);
    });
}

/// After a throw, `t` seconds after landing (the 96×96 die's timeline,
/// [`screens::pig_score`]). First the throw: its points as T1 (`+15`, gold
/// from 20) over what scored (`Strut`, `Twin`); an oops is red, the turn's
/// points lost (`−20`, or `0`) over `OOPS`; a smooch a red heart over the
/// banked score lost (`−40`, or `kiss`). Then the resting screen: the
/// player's token over the turn so far (`+35`), or after an oops or a
/// smooch, whose go it is now.
pub fn draw_pig_score<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    throw: &Throw,
    t: f32,
    next_player: u8,
    tokens: &[Token],
    alpha: f32,
) {
    use screens::pig_score::*;
    let token = |p: u8| tokens.get(p as usize).copied().unwrap_or(Token::default_for(p));
    let rest = ramp(t, PROMPT_AT, 0.3);
    let first = 1.0 - rest;
    in_pixels(c, |p| {
        // The throw.
        let pop = ramp(t, POP_AT, 0.3);
        let drop = roundf(4.0 * (1.0 - pop));
        let word_a = ramp(t, LABEL_AT, 0.25) * first;
        if pop > 0.0 && first > 0.0 {
            let a = alpha * pop * first;
            let (y1, y2) = table_layout(T1_NUM, true);
            match throw.outcome {
                Outcome::Score(n) => {
                    let colour = if n >= 20 { pal::GOLD } else { pal::WHITE };
                    draw_t1(p, &crate::table::signed(n as i32), y1 - drop, colour, a);
                    draw_t2(p, crate::table::throw_word(throw), y2, pal::WHITE, alpha * word_a);
                }
                Outcome::Bust => {
                    let lost = crate::table::signed(-(throw.turn_before as i32));
                    draw_t1(p, &lost, y1 - drop, pal::RED, a);
                    draw_t2(p, "OOPS", y2, pal::RED, alpha * word_a);
                }
                Outcome::Smooch => {
                    draw_centered(p, &spr::HEART, 0.0, y1 + 15.0 - drop, 3.0, style(pal::RED, a));
                    p.note_icon(24.0, a);
                    let word = if throw.banked_before > 0 {
                        crate::table::signed(-(throw.banked_before as i32))
                    } else {
                        let mut s = String::new();
                        let _ = s.push_str("kiss");
                        s
                    };
                    draw_t2(p, &word, y2, pal::RED, alpha * word_a);
                }
            }
        }

        // The resting screen.
        if rest <= 0.0 {
            return;
        }
        let a = alpha * rest;
        let (y1, y2) = table_layout(T1_TEXT, true);
        match throw.outcome {
            Outcome::Score(n) if !throw.won => {
                draw_t1_token(p, token(throw.player), y1, pal::PINK, a);
                let turn = crate::table::signed((throw.turn_before + n) as i32);
                draw_t2(p, &turn, y2, pal::WHITE, a);
            }
            Outcome::Score(_) => {}
            _ => {
                draw_t1_token(p, token(next_player), y1, pal::PINK, a);
                draw_t2(p, "turn", y2, pal::WHITE, a);
            }
        }
    });
}

/// A bank locking in, `t` seconds after the hold ([`screens::lock_in`]):
/// the banked points as T1 over a padlock (T2) that drops shut and turns
/// mint with a ring of sparks; the points become the player's new total
/// counting up; then whose go it is, as between turns.
pub fn draw_locked<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, l: &Locked, next: Token, t: f32, fade: f32) {
    use screens::lock_in::*;
    let alpha = ramp(t, 0.0, 0.15) * fade;
    let next_a = ramp(t, NEXT_AT, 0.3);
    in_pixels(c, |p| {
        if next_a >= 1.0 {
            let (y1, y2) = table_layout(T1_TEXT, true);
            draw_t1_token(p, next, y1, pal::PINK, alpha);
            draw_t2(p, "turn", y2, pal::WHITE, alpha);
            return;
        }
        let a = alpha * (1.0 - next_a);
        let (y1, y2) = table_layout(T1_NUM, true);
        let count = ramp(t, COUNT_AT, COUNT_S);
        let eased = 1.0 - (1.0 - count) * (1.0 - count);
        if t < COUNT_AT {
            draw_t1(p, &crate::table::signed(l.points as i32), y1, pal::WHITE, a);
        } else {
            let shown = l.before + roundf(l.points as f32 * eased) as u16;
            let colour = if count >= 1.0 { pal::MINT } else { pal::WHITE };
            let mut s: String<5> = String::new();
            let _ = write!(s, "{shown}");
            draw_t1(p, &s, y1, colour, a);
        }
        // The padlock, 13×15, as the T2.
        let closing = (t / SNAP_AT).clamp(0.0, 1.0);
        let lift = roundf(4.0 * (1.0 - closing * closing));
        let snapped = t >= SNAP_AT;
        let colour = if snapped { pal::MINT } else { pal::WHITE };
        draw_glyph(p, &spr::SHACKLE, -6.0, y2 - lift, 1.0, style(colour, a));
        draw_glyph(p, &spr::LOCK_BODY, -6.0, y2 + 6.0, 1.0, style(colour, a));
        p.note_icon(15.0, a);
        if snapped {
            let u = ((t - SNAP_AT) / 0.55).min(1.0);
            if u < 1.0 {
                for k in 0..8 {
                    let ang = k as f32 * core::f32::consts::PI / 4.0 + 0.4;
                    let r = 9.0 + 10.0 * u;
                    let (x, y) = (
                        roundf(libm::cosf(ang) * r),
                        roundf(y2 + 8.0 + libm::sinf(ang) * r),
                    );
                    p.fill_rect(x - 1.0, y - 1.0, 2.0, 2.0, style(pal::MINT, a * (1.0 - u)));
                }
            }
        }
    });
}

/// The win, `t` seconds in: the winner's token pops in gold as T1 with a
/// ring of gold sparks, over `wins`.
pub fn draw_pig_win<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    winner: Token,
    total: u16,
    t: f32,
    fade: f32,
) {
    let _ = total;
    let alpha = ramp(t, 0.0, 0.2) * fade;
    let (y1, y2) = table_layout(T1_TEXT, true);
    in_pixels(c, |p| {
        let pop = ramp(t, 0.0, 0.45);
        let drop = roundf(6.0 * (1.0 - pop));
        draw_t1_token(p, winner, y1 - drop, pal::GOLD, alpha * pop);
        let u = ((t - 0.35) / 0.7).clamp(0.0, 1.0);
        if t > 0.35 && u < 1.0 {
            for k in 0..10 {
                let a = k as f32 * core::f32::consts::PI / 5.0 + 0.2;
                let r = 16.0 + 12.0 * u;
                let (x, y) = (roundf(libm::cosf(a) * r), roundf(y1 + 10.0 + libm::sinf(a) * r));
                p.fill_rect(x - 1.0, y - 1.0, 2.0, 2.0, style(pal::GOLD, alpha * (1.0 - u)));
            }
        }
        draw_t2(p, "wins", y2, pal::WHITE, alpha * ramp(t, 0.6, 0.3));
    });
}

// ---------- Hot Potato ----------
//
// An arcade game on a 64×64 screen: the potato is a character. Lit, it
// goes from calm to worried to panicking as the ticks speed up (their pace,
// not the time left: nobody can count it down). It reddens, shakes harder,
// sweats and steams, its fuse fizzing, over a flashing drop-shadowed
// "PASS IT!" and an eight-block heat bar. When it goes off: a white flash,
// an 8-bit fireball, chunks flying, and the potato left charred with X
// eyes under a bouncing BOOM.

/// `a` to `b` by `t` (0–1).
fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t + 0.5) as u8;
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

/// The fractional part of `x`.
fn frac(x: f32) -> f32 {
    x - floorf(x)
}

/// A potato's look: its skin colour and face.
struct Spud {
    skin: Color,
    face: &'static Glyph<13>,
}

/// The potato at 2×, its top left at `(x, y)`, with its fuse; the fuse's
/// tip fizzes if `lit` (`t` drives the flicker).
fn draw_potato<T: Target>(p: &mut Painter<T>, x: f32, y: f32, spud: &Spud, lit: Option<f32>, alpha: f32) {
    // The fuse: a short curl up and to the right of the top.
    let (fx, fy) = (x + spr::POTATO_FUSE.0 * 2.0, y + spr::POTATO_FUSE.1 * 2.0);
    let fuse = style(if lit.is_some() { pal::EMBER } else { pal::DIM }, alpha);
    for (dx, dy) in [(0.0, -2.0), (2.0, -4.0), (2.0, -6.0), (4.0, -8.0)] {
        p.fill_rect(fx + dx, fy + dy, 2.0, 2.0, fuse);
    }
    let dark = mix(pal::POTATO_DARK, spud.skin, 0.25);
    draw_glyph(p, &spr::POTATO_SKIN, x, y, 2.0, style(spud.skin, alpha));
    draw_glyph(p, &spr::POTATO_SPOTS, x, y, 2.0, style(dark, alpha));
    draw_glyph(p, spud.face, x, y, 2.0, style(pal::POTATO_DARK, alpha));
    if let Some(t) = lit {
        // The spark flickers between its two shapes and jumps about a
        // pixel, every frame or two.
        let k = floorf(t * 24.0) as i32;
        let g = if k & 1 == 0 {
            &spr::SPARK_X
        } else {
            &spr::SPARK_PLUS
        };
        let (jx, jy) = [(0.0, 0.0), (1.0, -1.0), (0.0, -1.0), (1.0, 0.0)][(k & 3) as usize];
        let c = if k % 3 == 0 { pal::WHITE } else { pal::GOLD };
        draw_centered(p, g, fx + 5.0 + jx, fy - 10.0 + jy, 1.0, style(c, alpha));
    }
}

/// The potato at 2× is T1: 32×26, its fuse curling up above the right of
/// its top. Its top, for a table screen with T2 under it.
const SPUD_H: f32 = 26.0;

/// Hot Potato at rest: a calm potato with its fuse out as T1, `ready` as T2.
pub fn draw_potato_label<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
    let (y1, y2) = table_layout(SPUD_H, true);
    in_pixels(c, |p| {
        let calm = Spud {
            skin: pal::POTATO,
            face: &spr::FACE_CALM,
        };
        draw_potato(p, -16.0, y1 + 2.0, &calm, None, alpha);
        p.note_icon(SPUD_H, alpha);
        draw_t2(p, "ready", y2 + 2.0, pal::WHITE, alpha);
    });
}

/// A lit fuse, `t` seconds on the die's clock: the potato heating with
/// `heat` (0–1), flaring on each tick (`pulse`, 0–1).
pub fn draw_fuse<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, heat: f32, pulse: f32, t: f32) {
    in_pixels(c, |p| {
        let face = if heat < 0.35 {
            &spr::FACE_CALM
        } else if heat < 0.7 {
            &spr::FACE_WORRIED
        } else {
            &spr::FACE_PANIC
        };
        let hot = mix(pal::POTATO, pal::RED, heat * 0.7);
        let spud = Spud {
            skin: mix(hot, pal::WHITE, 0.4 * pulse),
            face,
        };
        // It shakes harder the hotter it gets, in whole pixels.
        let amp = if heat < 0.35 {
            0.0
        } else if heat < 0.7 {
            1.0
        } else {
            2.0
        };
        let (dx, dy) = (
            roundf(sinf(t * 47.0) * amp),
            roundf(sinf(t * 31.0 + 1.0) * amp * 0.5),
        );
        let (x, y) = (-16.0 + dx, -22.0 + dy);
        // Steam rising off it once it's warm.
        if heat > 0.3 {
            for k in 0..3 {
                let u = frac(t * 0.9 + k as f32 / 3.0);
                let sx = roundf(-9.0 + 9.0 * k as f32 + sinf(u * 7.0 + k as f32) * 2.0);
                let sy = roundf(-20.0 - 9.0 * u);
                let a = (1.0 - u) * ((heat - 0.3) / 0.3).min(1.0) * 0.8;
                p.fill_rect(sx, sy, 2.0, 2.0, style(pal::DIM, a));
            }
        }
        draw_potato(p, x, y, &spud, Some(t), 1.0);
        p.note_icon(SPUD_H, 1.0);
        // Sweat flying off its sides: more, and faster, the worse it gets.
        if heat > 0.35 {
            let n = if heat < 0.7 { 2 } else { 4 };
            let rate = if heat < 0.7 { 1.2 } else { 2.0 };
            for k in 0..n {
                let side = if k % 2 == 0 { -1.0 } else { 1.0 };
                let u = frac(t * rate + k as f32 * 0.37);
                let sx = roundf(side * (15.0 + 8.0 * u)) - 1.0;
                let sy = roundf(-14.0 + 4.0 * (k / 2) as f32 - 4.0 * u + 14.0 * u * u);
                draw_glyph(p, &spr::SWEAT, sx, sy, 1.0, style(pal::SWEAT, 1.0 - u));
            }
        }
        // PASS flashes white and gold, faster as it heats (T2).
        let flash = frac(t * (1.5 + 3.5 * heat)) < 0.5;
        let colour = if flash { pal::WHITE } else { pal::GOLD };
        draw_t2(p, "PASS", 6.0, colour, 1.0);
        // The heat bar: eight blocks, mint to gold to red; the newest one
        // flashes on each tick.
        let lit = libm::ceilf(heat * 8.0) as usize;
        for i in 0..8 {
            let bx = -23.0 + i as f32 * 6.0;
            let on = i < lit;
            let base = match i {
                0..=2 => pal::MINT,
                3..=5 => pal::GOLD,
                _ => pal::RED,
            };
            let colour = if !on {
                pal::FAINT
            } else if i + 1 == lit {
                mix(base, pal::WHITE, pulse)
            } else {
                base
            };
            p.fill_rect(bx, 24.0, 5.0, 3.0, style(colour, 1.0));
        }
    });
}

/// The fuse ran out, `t` seconds ago: a white flash, an 8-bit fireball and
/// flying chunks, then the potato charred under a bouncing BOOM, and a tap
/// to reset. Fades as the 96×96 one.
pub fn draw_boom<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, t: f32) {
    let fade = (1.0 - (t - 4.6) / 1.0).clamp(0.0, 1.0);
    in_pixels(c, |p| {
        if t < 0.1 {
            p.fill_rect(-32.0, -32.0, 64.0, 64.0, style(pal::WHITE, 1.0 - t / 0.1));
        }
        // The fireball: 2×2 blocks out to a ragged edge that grows, then
        // burns out from the middle. White at the heart, red at the rim.
        if t < 0.7 {
            let u = t / 0.7;
            let grow = 1.0 - (u * 2.0).min(1.0);
            let r = 26.0 * (1.0 - grow * grow);
            let hole = r * ((u - 0.3) / 0.7).max(0.0) * 1.1;
            let mut y = -26.0;
            while y < 26.0 {
                let mut x = -26.0;
                while x < 26.0 {
                    let (cx, cy) = (x + 1.0, y + 1.0);
                    let d = libm::sqrtf(cx * cx + cy * cy);
                    // A fixed ragged edge from the block's position.
                    let h = frac(sinf(x * 12.9898 + y * 78.233) * 43758.547);
                    let edge = r * (0.8 + 0.25 * h);
                    if d < edge && d >= hole {
                        let k = d / edge.max(1.0) + 0.6 * u;
                        let colour = if k < 0.35 {
                            pal::WHITE
                        } else if k < 0.6 {
                            pal::GOLD
                        } else if k < 0.85 {
                            pal::EMBER
                        } else {
                            pal::RED
                        };
                        p.fill_rect(x, y, 2.0, 2.0, style(colour, 1.0));
                    }
                    x += 2.0;
                }
                y += 2.0;
            }
        }
        // Chunks of potato fly out and fall. Like the smoke, they run on
        // past the glass's corners.
        if t < 1.3 {
            for k in 0..12 {
                let a = k as f32 * core::f32::consts::TAU / 12.0 + 0.3;
                let v = 34.0 + 10.0 * (k % 3) as f32;
                let x = roundf(libm::cosf(a) * v * t);
                let y = roundf(libm::sinf(a) * v * t + 40.0 * t * t);
                let size = if t < 0.5 { 3.0 } else { 2.0 };
                let colour = if k % 2 == 0 { pal::POTATO } else { pal::EMBER };
                p.fill_rect(
                    x - 1.0,
                    y - 1.0,
                    size,
                    size,
                    style(colour, fade * (1.0 - t / 1.3)),
                );
            }
        }
        // What's left: a charred potato, smoking.
        let left = ((t - 0.45) / 0.3).clamp(0.0, 1.0) * fade;
        if left > 0.0 {
            let burnt = Spud {
                skin: pal::CHAR,
                face: &spr::FACE_BURNT,
            };
            draw_potato(p, -16.0, -21.0, &burnt, None, left);
            p.note_icon(SPUD_H, left);
            for k in 0..3 {
                let u = frac(t * 0.6 + k as f32 / 3.0);
                let sx = roundf(-6.0 + 6.0 * k as f32 + sinf(u * 6.0 + k as f32) * 2.0);
                let sy = roundf(-26.0 - 8.0 * u);
                p.fill_rect(sx, sy, 2.0, 2.0, style(pal::DIM, left * (1.0 - u) * 0.7));
            }
        }
        // BOOM (T2) drops in from the top and bounces to rest.
        if t > 0.2 {
            let u = ((t - 0.2) / 0.5).min(1.0);
            let bounce = if u < 0.6 {
                let k = u / 0.6;
                -40.0 * (1.0 - k * k)
            } else {
                let k = (u - 0.6) / 0.4;
                -6.0 * sinf(k * core::f32::consts::PI)
            };
            draw_t2(p, "BOOM", roundf(10.0 + bounce), pal::RED, fade);
        }
    });
}

// ---------- Pass the Pot ----------
//
// The bills in hand are banknotes in mint (money you hold), the ones you
// don't roll faint, with the count beside the word and a tap hint. A
// result shows each die as a sprite: arrows pass a bill left or right
// (white), the pot takes one (gold, like the coins), a dot keeps it
// (mint). Under them, one short line per kind in the same colour, so no
// line runs off the face however the dice fall.

/// Pass the Pot between rolls: how many bills are in hand as T1, in mint
/// (money you hold), and `bills` as T2.
pub fn draw_bills<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, n: u8, alpha: f32) {
    let mut digits: String<2> = String::new();
    let _ = write!(digits, "{n}");
    let word = if n == 1 { "bill" } else { "bills" };
    let (y1, y2) = table_layout(T1_NUM, true);
    in_pixels(c, |p| {
        draw_t1(p, &digits, y1, pal::MINT, alpha);
        draw_t2(p, word, y2, pal::WHITE, alpha);
    });
}

/// What a Pass the Pot die says, and its colour.
fn pot_face(v: u8) -> (PotFace, Color) {
    let f = PotFace::from_raw(v);
    let colour = match f {
        PotFace::Left | PotFace::Right => pal::WHITE,
        PotFace::Pot => pal::GOLD,
        PotFace::Keep => pal::MINT,
    };
    (f, colour)
}

/// A Pass the Pot die as a T1 glyph, `w` × `h` with its top at `y`: an
/// arrow passing a bill left or right, the pot, or a dot to keep it.
#[allow(clippy::too_many_arguments)]
fn draw_pot_glyph<T: Target>(
    p: &mut Painter<T>,
    face: PotFace,
    cx: f32,
    y: f32,
    w: f32,
    h: f32,
    colour: Color,
    alpha: f32,
) {
    let s = style(colour, alpha);
    let (l, r, mid) = (cx - floorf(w / 2.0), cx - floorf(w / 2.0) + w, y + h / 2.0);
    match face {
        PotFace::Left | PotFace::Right => {
            let head = roundf(w * 0.55);
            let shaft = roundf(h * 0.34);
            let (tip, base, end) = if face == PotFace::Left {
                (l, l + head, r)
            } else {
                (r, r - head, l)
            };
            p.fill_triangle([(tip, mid), (base, y), (base, y + h)], s);
            let (x0, x1) = if base < end {
                (base - 1.0, end)
            } else {
                (end, base + 1.0)
            };
            p.fill_rect(x0, roundf(mid - shaft / 2.0), x1 - x0, shaft, s);
        }
        PotFace::Pot => {
            // A cauldron: rim, round body, and the pot's mouth darker.
            let rim = roundf(h * 0.2);
            let top = y + roundf(h * 0.18);
            p.fill_rect(l, top, w, rim, s);
            let body_r = floorf(w * 0.42);
            p.fill_circle(cx, y + h - body_r, body_r, s);
            p.fill_rect(
                cx - body_r,
                top + rim,
                2.0 * body_r,
                h - body_r - rim - roundf(h * 0.18),
                s,
            );
            p.fill_rect(l + 2.0, top + 2.0, w - 4.0, rim - 4.0, style(colour, alpha * 0.3));
        }
        PotFace::Keep => {
            p.fill_circle(cx, mid, floorf(w.min(h) * 0.32), s);
        }
    }
}

/// A Pass the Pot result: the dice as T1 glyphs (bigger the fewer there
/// are), and as T2 `keep` or how many bills leave the hand, `−2`. A dud
/// greys it all.
fn draw_pot_result<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, record: &RollRecord, dud: bool, alpha: f32) {
    let values = record.values.as_slice();
    let n = values.len().max(1);
    let (w, h) = match n {
        1 => (30.0, 30.0),
        2 => (24.0, 26.0),
        _ => (16.0, 22.0),
    };
    let word = crate::table::pot(values);
    let (y1, y2) = table_layout(h, true);
    in_pixels(c, |p| {
        let pitch = w + 5.0;
        for (i, &v) in values.iter().enumerate() {
            let x = roundf((i as f32 - (n as f32 - 1.0) / 2.0) * pitch);
            let (face, colour) = pot_face(v);
            let colour = if dud { pal::DIM } else { colour };
            draw_pot_glyph(p, face, x, y1, w, h, colour, alpha);
        }
        p.note_icon(h, alpha);
        let colour = if dud {
            pal::DIM
        } else if word.as_str() == "keep" {
            pal::MINT
        } else {
            pal::WHITE
        };
        draw_t2(p, &word, y2, colour, alpha);
    });
}

// ---------- the hold ring ----------

/// The hold ring: a rounded square two pixels wide, set in from the panel's
/// edge and round its corners with the glass's own radius, so it sits
/// evenly inside the lit area. It fills clockwise from 12 o'clock as a hold
/// progresses (`p`, 0–1), in whole pixels: a 96×96 ring's anti-aliased
/// line goes soft at this size. `grow` (canvas units) pushes it outward as
/// it flashes away.
/// What a hold will do, on the held face while the ring fills (brief 3,
/// 2.2.1): the action in a word (H2) over what it comes to (H1), or the
/// word alone as H1.
pub fn draw_hold_preview<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, word: &str, value: &str) {
    in_pixels(c, |p| {
        if value.is_empty() {
            H1.draw(p, word, 0.0, -9.0, 1.0, Align::Center, style(pal::WHITE, 1.0));
        } else {
            TEXT.draw(p, word, 0.0, -15.0, 1.0, Align::Center, style(pal::DIM, 1.0));
            H1.draw(p, value, 0.0, -4.0, 1.0, Align::Center, style(pal::WHITE, 1.0));
        }
    });
}

/// A tap's hint along the top of the face, `hold: bank`; the face's own
/// content moves down to make room ([`crate::target::DisplayTarget::HINT_ROOM`]).
pub fn draw_tap_hint<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, text: &str, alpha: f32) {
    in_pixels(c, |p| {
        TEXT.draw(p, text, 0.0, -30.0, 1.0, Align::Center, style(pal::DIM, alpha));
    });
}

pub fn draw_hold_ring<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, p: f32, alpha: f32, grow: f32) {
    if alpha <= 0.0 || p <= 0.0 {
        return;
    }
    let grow = grow * crate::gfx::k_of::<T>();
    let half = T::WIDTH as f32 / 2.0;
    // The centreline, from the middle; its corners' radius.
    let h = half - 2.0 + grow;
    let r = (T::MASK_RADIUS_PX - 2.0 + grow).max(2.0);
    let a = h - r;
    let quarter = core::f32::consts::FRAC_PI_2 * r;
    let total = 8.0 * a + 4.0 * quarter;
    let lit = p.min(1.0) * total;
    in_pixels(c, |pt| {
        let st = style(pal::WHITE, alpha);
        let n = T::WIDTH as i32;
        for j in 0..n {
            for i in 0..n {
                // This pixel's centre.
                let (x, y) = (i as f32 - half + 0.5, j as f32 - half + 0.5);
                let (qx, qy) = (x.clamp(-a, a), y.clamp(-a, a));
                let (dx, dy) = (x - qx, y - qy);
                let d = libm::sqrtf(dx * dx + dy * dy) - r;
                if !(-1.0..1.0).contains(&d) {
                    continue;
                }
                let th = libm::atan2f(dy, dx);
                use core::f32::consts::PI;
                // How far round the centreline, clockwise from 12 o'clock.
                let s = match (x > a, x < -a, y > a, y < -a) {
                    (true, _, _, true) => a + r * (th + PI / 2.0),
                    (true, _, true, _) => 3.0 * a + quarter + r * th,
                    (_, true, true, _) => 5.0 * a + 2.0 * quarter + r * (th - PI / 2.0),
                    (_, true, _, true) => 7.0 * a + 3.0 * quarter + r * (th + PI),
                    (true, _, _, _) => a + quarter + (y + a),
                    (_, _, true, _) => 3.0 * a + 2.0 * quarter + (a - x),
                    (_, true, _, _) => 5.0 * a + 3.0 * quarter + (a - y),
                    _ if x >= 0.0 => x,
                    _ => total + x,
                };
                if s <= lit {
                    pt.fill_rect(x - 0.5, y - 0.5, 1.0, 1.0, st);
                }
            }
        }
    });
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

    /// Every menu page, every value: a few drafts' rings, tipped through.
    fn every_page() -> std::vec::Vec<Draft> {
        use crate::menu::{Held, PlayMode, Settings};
        let mut drafts = std::vec::Vec::new();
        for play in PlayMode::ALL {
            drafts.push(Draft::new(&Settings {
                play,
                players: 6,
                ..Settings::default()
            }));
        }
        let apps = Draft::new(&Settings::default()).tipped(crate::tips::TipDir::Right);
        if let Held::Next(settings) = apps.tipped(crate::tips::TipDir::Down).held() {
            drafts.push(settings);
        }
        drafts.push(Draft::alone(&Settings::default(), Page::Next));
        let mut out = std::vec::Vec::new();
        for d in drafts {
            for i in 0..d.ring().len() as i32 {
                for v in 0..10 {
                    out.push(
                        d.stepped(crate::tips::TipDir::Left, i)
                            .stepped(crate::tips::TipDir::Up, v),
                    );
                }
            }
        }
        out
    }

    #[test]
    fn every_menu_page_fits() {
        for d in every_page() {
            let view = d.view(100);
            for line in [
                view.status.as_str(),
                view.title.as_str(),
                view.caption.unwrap_or(""),
            ] {
                assert!(TEXT.measure(line) <= LINE_MAX, "{line:?} on {:?}", d.page);
            }
            if let crate::menu::Value::Text(t) = &view.value {
                assert!(H1.measure(t) <= LINE_MAX, "{t:?} on {:?}", d.page);
            }
        }
    }

    #[test]
    fn every_table_word_fits_a_line() {
        use smokebomb_shared::types::MAX_DICE;
        for die in DieKind::NUMERIC {
            for n in 1..=MAX_DICE as u8 {
                let max = crate::table::total(die.sides() as u16 * n as u16);
                let w = if max.chars().all(|c| c.is_ascii_digit()) {
                    NUM_M.measure(&max)
                } else {
                    T1.measure(&max)
                };
                assert!(w <= LINE_MAX, "{n}{die:?}: total {max}");
                let dice = crate::table::dice(die, n);
                assert!(T2.measure(&dice) <= LINE_MAX, "{dice}");
            }
        }
        for word in [
            "MAX", "DUD", "bills", "ready", "PASS", "BOOM", "wins", "turn", "low", "keep", "−3", "OOPS",
            "kiss", "−99", "+60", "Twin", "Nap", "Belly", "Strut", "Dive", "Tipsy",
        ] {
            assert!(T2.measure(word) <= LINE_MAX, "{word}");
        }
        for ch in 'A'..='Z' {
            let mut s: String<2> = String::new();
            let _ = s.push(ch);
            assert!(T1.measure(&s) <= LINE_MAX, "{ch}");
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
    fn face_numbers_are_a_die() {
        // Opposite faces (+X/−X, +Y/−Y, +Z/−Z) sum to 7.
        for f in [0, 2, 4] {
            assert_eq!(face_number(f) + face_number(f + 1), 7);
        }
    }
}
