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

use crate::font64::{draw_centered, draw_glyph, Align, BitFont, Glyph, NUM_L, NUM_M, NUM_S, TAG, TEXT};
use crate::gfx::{Painter, Style};
use crate::menu::{Draft, Page, Setup};
use crate::nest::{ChargeView, Label, NestFace, Screen};
use crate::palette64 as pal;
use crate::pigs::{throw_label, Locked, Outcome, Symbol, Throw, Token};
use crate::screens::{self, setup_label, Ctx, FACE_START};
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
/// tag; a fumble (every die a 1) turns it red with `DUD`. Pass the Pot shows
/// its dice as sprites ([`draw_pot_result`]).
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
/// this is in a small tag. Pass the Pot shows the bills in hand, and Hot
/// Potato its potato; Pig Toss has its own label ([`draw_pigs_label`]).
pub fn draw_idle<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    setup: Setup,
    label: &str,
    alpha: f32,
    face: usize,
) {
    let die = match setup {
        Setup::Roll(die, _) if die.is_numeric() => die,
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

/// The boot on one face: the 96×96 die's animation, drawn smooth through
/// the 64×64 transform: the pips slide and grow between values, the top
/// face's centre pip squares up into a sugar cube that wiggles and bursts
/// into crystals, and `Sugarcube` writes itself on in the script with a
/// glint riding the pen. In colour: white pips and cube, sugar crystals and
/// a gold glint.
pub fn draw_boot<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, index: usize, top: bool, t: f32) {
    let ink = screens::BootInk {
        pips: pal::WHITE,
        crystals: pal::SUGAR,
        word: pal::WHITE,
        glint: pal::GOLD,
    };
    screens::draw_boot_in(c, index, top, t, ink);
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
        Page::Token(i) => {
            // A player's initial or symbol, big and pink.
            draw_token(
                p,
                m.tokens[i as usize],
                0.0,
                -4.0,
                2.0,
                pal::PINK,
                alpha,
                Align::Center,
            );
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

/// Draw a player's initial or symbol, the top of its 7 rows at `y`.
#[allow(clippy::too_many_arguments)]
fn draw_token<T: Target>(
    p: &mut Painter<T>,
    token: Token,
    x: f32,
    y: f32,
    scale: f32,
    colour: Color,
    alpha: f32,
    align: Align,
) {
    let w = token_width(token) as f32 * scale;
    let left = match align {
        Align::Left => x,
        Align::Center => x - floorf(token_width(token) as f32 / 2.0) * scale,
        Align::Right => x - w,
    };
    match token.as_symbol() {
        Some(sym) => draw_glyph(p, symbol_sprite(sym), left, y, scale, style(colour, alpha)),
        None => {
            let mut s: String<2> = String::new();
            let _ = s.push(token.initial().unwrap_or('?'));
            TEXT.draw(p, &s, left, y, scale, Align::Left, style(colour, alpha));
        }
    }
}

/// A line of 5×7 text with a token in it, centred on `cx`: `before`, the
/// token (pink), `after`.
#[allow(clippy::too_many_arguments)]
fn token_line<T: Target>(
    p: &mut Painter<T>,
    before: &str,
    token: Token,
    after: &str,
    cx: f32,
    y: f32,
    text: Color,
    alpha: f32,
) {
    let gap = |s: &str| if s.is_empty() { 0 } else { TEXT.gap as usize };
    let wb = TEXT.measure(before);
    let wa = TEXT.measure(after);
    let w = wb + gap(before) + token_width(token) + gap(after) + wa;
    let mut x = cx - floorf(w as f32 / 2.0);
    if !before.is_empty() {
        TEXT.draw(p, before, x, y, 1.0, Align::Left, style(text, alpha));
        x += (wb + gap(before)) as f32;
    }
    draw_token(p, token, x, y, 1.0, pal::PINK, alpha, Align::Left);
    x += (token_width(token) + gap(after)) as f32;
    if !after.is_empty() {
        TEXT.draw(p, after, x, y, 1.0, Align::Left, style(text, alpha));
    }
}

/// The happy pig, 19×16 at `scale`, its top-left at `(x, y)`.
fn draw_pig_face<T: Target>(p: &mut Painter<T>, x: f32, y: f32, scale: f32, alpha: f32) {
    draw_glyph(p, &spr::PIG_FACE, x, y, scale, style(pal::PINK, alpha));
    draw_glyph(p, &spr::PIG_SNOUT, x, y, scale, style(pal::PINK_LIGHT, alpha));
    draw_glyph(p, &spr::PIG_DARK, x, y, scale, style(pal::PINK_DARK, alpha));
}

/// A number in 30 px numerals with an optional plus in front, centred on
/// `cx`. The plus is drawn to match: 14 px arms, 4 px stems.
fn draw_points<T: Target>(p: &mut Painter<T>, n: u16, plus: bool, cx: f32, y: f32, s: Style) {
    let mut digits: String<6> = String::new();
    let _ = write!(digits, "{n}");
    let (arm, stem, gap) = (14.0, 4.0, 3.0);
    let wd = NUM_M.measure(&digits) as f32;
    let w = wd + if plus { arm + gap } else { 0.0 };
    let x0 = cx - floorf(w / 2.0);
    if plus {
        let mid = y + 15.0;
        p.fill_rect(x0, mid - stem / 2.0, arm, stem, s);
        p.fill_rect(x0 + (arm - stem) / 2.0, mid - arm / 2.0, stem, arm, s);
    }
    let dx = x0 + if plus { arm + gap } else { 0.0 };
    NUM_M.draw(p, &digits, dx + floorf(wd / 2.0), y, s);
}

/// 0 before `at`, rising to 1 over `over` seconds.
fn ramp(t: f32, at: f32, over: f32) -> f32 {
    ((t - at) / over).clamp(0.0, 1.0)
}

/// Between turns: a happy pig and whose go it is (or who won).
pub fn draw_pigs_label<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, token: Token, won: bool, alpha: f32) {
    in_pixels(c, |p| {
        draw_pig_face(p, -19.0, -28.0, 2.0, alpha);
        if won {
            token_line(p, "", token, " wins!", 0.0, 10.0, pal::GOLD, alpha);
        } else {
            token_line(p, "", token, " to roll", 0.0, 10.0, pal::WHITE, alpha);
        }
    });
}

/// After a throw, `t` seconds after landing (the 96×96 die's timeline,
/// [`screens::pig_score`]): the throw's points (or OOPS, or a smooch's
/// heart) with the pose under them, then the resting screen.
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
    let lost = !matches!(throw.outcome, Outcome::Score(_));
    let rest = ramp(t, PROMPT_AT, 0.3);
    let first = 1.0 - rest;
    in_pixels(c, |p| {
        // The throw.
        let pop = ramp(t, POP_AT, 0.3);
        let drop = roundf(4.0 * (1.0 - pop));
        let label_a = ramp(t, LABEL_AT, 0.25) * first;
        if pop > 0.0 && first > 0.0 {
            let a = alpha * pop * first;
            match throw.outcome {
                Outcome::Score(n) => {
                    let colour = if n >= 20 { pal::GOLD } else { pal::WHITE };
                    draw_points(p, n, true, 0.0, -24.0 - drop, style(colour, a));
                }
                Outcome::Bust => {
                    TEXT.draw(
                        p,
                        "OOPS",
                        0.0,
                        -16.0 - drop,
                        2.0,
                        Align::Center,
                        style(pal::RED, a),
                    );
                }
                Outcome::Smooch => {
                    draw_centered(p, &spr::HEART, 0.0, -18.0 - drop, 2.0, style(pal::RED, a));
                    TEXT.draw(p, "SMOOCH", 0.0, -4.0, 1.0, Align::Center, style(pal::RED, a));
                }
            }
        }
        if label_a > 0.0 {
            let a = alpha * label_a;
            let mut label: String<32> = String::new();
            let colour = match throw.outcome {
                Outcome::Smooch if throw.banked_before > 0 && t >= TURN_AT => {
                    // The banked score counts down to nothing.
                    let k = ramp(t, TURN_AT, COUNT_S);
                    let shown = roundf(throw.banked_before as f32 * (1.0 - k * k)) as u16;
                    let _ = write!(label, "Score {shown}");
                    pal::RED
                }
                Outcome::Smooch => {
                    let _ = write!(label, "Pigs touched!");
                    pal::DIM
                }
                Outcome::Bust if throw.turn_before > 0 => {
                    let _ = write!(label, "Lost {}", throw.turn_before);
                    pal::DIM
                }
                Outcome::Bust => {
                    let _ = write!(label, "Nothing lost");
                    pal::DIM
                }
                Outcome::Score(_) => {
                    let _ = write!(label, "{}", throw_label(throw.poses, false));
                    pal::DIM
                }
            };
            let y = if lost { 6.0 } else { 10.0 };
            draw_lines(p, &label, 0.0, y, colour, a);
        }

        // The resting screen.
        if rest <= 0.0 {
            return;
        }
        let a = alpha * rest;
        if lost {
            TEXT.draw(p, "Pass to", 0.0, -27.0, 1.0, Align::Center, style(pal::DIM, a));
            draw_token(
                p,
                token(next_player),
                0.0,
                -15.0,
                3.0,
                pal::PINK,
                a,
                Align::Center,
            );
            draw_hint_roll(p, 0.0, 16.0, a);
        } else {
            let turn = match throw.outcome {
                Outcome::Score(n) => throw.turn_before + n,
                _ => 0,
            };
            token_line(p, "", token(throw.player), "'s turn", 0.0, -28.0, pal::DIM, a);
            draw_points(p, turn, false, 0.0, -17.0, style(pal::WHITE, a));
            // Roll again, or bank for this.
            let total = throw.banked_before + turn;
            let mut bank: String<6> = String::new();
            let _ = write!(bank, "{total}");
            let w_roll = spr::SHAKE.width as usize + 2 + TEXT.measure("roll");
            let w_bank = 5 + 2 + TEXT.measure(&bank);
            let x0 = -floorf((w_roll + 8 + w_bank) as f32 / 2.0);
            draw_hint_roll(p, x0 + floorf(w_roll as f32 / 2.0), 17.0, a);
            let xb = x0 + (w_roll + 8) as f32;
            draw_glyph(p, &LOCK_SMALL, xb, 17.0, 1.0, style(pal::MINT, a));
            TEXT.draw(p, &bank, xb + 7.0, 17.0, 1.0, Align::Left, style(pal::MINT, a));
        }
    });
}

/// A 5×7 padlock for the bank hint.
static LOCK_SMALL: Glyph<7> = crate::font64::glyph(' ', ".###. #...# #...# ##### ##.## ##.## #####");

/// "Shake: roll", as an icon and a word, centred on `cx`, top at `y`.
fn draw_hint_roll<T: Target>(p: &mut Painter<T>, cx: f32, y: f32, alpha: f32) {
    let w = spr::SHAKE.width as usize + 2 + TEXT.measure("roll");
    let x = cx - floorf(w as f32 / 2.0);
    draw_glyph(p, &spr::SHAKE, x, y + 1.0, 1.0, style(pal::DIM, alpha));
    TEXT.draw(
        p,
        "roll",
        x + (spr::SHAKE.width + 2) as f32,
        y,
        1.0,
        Align::Left,
        style(pal::WHITE, alpha),
    );
}

/// A bank locking in, `t` seconds after the tap ([`screens::lock_in`]): the
/// padlock drops shut and turns mint with a ring of sparks, the points
/// become the player's new total counting up, then whose turn is next.
pub fn draw_locked<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, l: &Locked, next: Token, t: f32, fade: f32) {
    use screens::lock_in::*;
    let alpha = ramp(t, 0.0, 0.15) * fade;
    in_pixels(c, |p| {
        let closing = (t / SNAP_AT).clamp(0.0, 1.0);
        let lift = roundf(5.0 * (1.0 - closing * closing));
        let snapped = t >= SNAP_AT;
        let colour = if snapped { pal::MINT } else { pal::WHITE };
        draw_glyph(p, &spr::SHACKLE, -6.0, -29.0 - lift, 1.0, style(colour, alpha));
        draw_glyph(p, &spr::LOCK_BODY, -6.0, -24.0, 1.0, style(colour, alpha));
        if snapped {
            let u = ((t - SNAP_AT) / 0.55).min(1.0);
            if u < 1.0 {
                for k in 0..8 {
                    let a = k as f32 * core::f32::consts::PI / 4.0 + 0.4;
                    let r = 12.0 + 14.0 * u;
                    let (x, y) = (roundf(libm::cosf(a) * r), roundf(-20.0 + libm::sinf(a) * r));
                    p.fill_rect(x - 1.0, y - 1.0, 2.0, 2.0, style(pal::MINT, alpha * (1.0 - u)));
                }
            }
        }
        let count = ramp(t, COUNT_AT, COUNT_S);
        let eased = 1.0 - (1.0 - count) * (1.0 - count);
        if t < COUNT_AT {
            draw_points(p, l.points, true, 0.0, -10.0, style(pal::WHITE, alpha));
        } else {
            let shown = l.before + roundf(l.points as f32 * eased) as u16;
            let colour = if count >= 1.0 { pal::MINT } else { pal::WHITE };
            draw_points(p, shown, false, 0.0, -10.0, style(colour, alpha));
        }
        let next_a = ramp(t, NEXT_AT, 0.3);
        if next_a > 0.0 {
            token_line(p, "", next, " to roll", 0.0, 22.0, pal::WHITE, alpha * next_a);
        }
    });
}

/// The win, `t` seconds in: the happy pig bounces in with gold sparks, then
/// who won, and how to start again.
pub fn draw_pig_win<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    winner: Token,
    total: u16,
    t: f32,
    fade: f32,
) {
    let _ = total;
    let alpha = ramp(t, 0.0, 0.2) * fade;
    in_pixels(c, |p| {
        let pop = ramp(t, 0.0, 0.45);
        let bob = if pop >= 1.0 {
            roundf(sinf((t - 0.45) * 3.5))
        } else {
            roundf(8.0 * (1.0 - pop))
        };
        draw_pig_face(p, -19.0, -30.0 + bob, 2.0, alpha * pop);
        let u = ((t - 0.35) / 0.7).clamp(0.0, 1.0);
        if t > 0.35 && u < 1.0 {
            for k in 0..10 {
                let a = k as f32 * core::f32::consts::PI / 5.0 + 0.2;
                let r = 20.0 + 12.0 * u;
                let (x, y) = (roundf(libm::cosf(a) * r), roundf(-14.0 + libm::sinf(a) * r));
                p.fill_rect(x - 1.0, y - 1.0, 2.0, 2.0, style(pal::GOLD, alpha * (1.0 - u)));
            }
        }
        let words = ramp(t, 0.6, 0.3);
        if words > 0.0 {
            token_line(p, "", winner, " wins!", 0.0, 7.0, pal::GOLD, alpha * words);
        }
        let prompt = ramp(t, 1.6, 0.3);
        if prompt > 0.0 {
            let w = spr::TAP.width as usize + 2 + TEXT.measure("new game");
            let x = -floorf(w as f32 / 2.0);
            draw_glyph(p, &spr::TAP, x, 17.0, 1.0, style(pal::DIM, alpha * prompt));
            TEXT.draw(
                p,
                "new game",
                x + (spr::TAP.width + 2) as f32,
                17.0,
                1.0,
                Align::Left,
                style(pal::WHITE, alpha * prompt),
            );
        }
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

/// A short line of arcade text: a one-pixel shadow down and right, then
/// the text.
fn draw_arcade<T: Target>(
    p: &mut Painter<T>,
    text: &str,
    y: f32,
    scale: f32,
    colour: Color,
    shadow: Color,
    alpha: f32,
) {
    TEXT.draw(
        p,
        text,
        scale,
        y + scale,
        scale,
        Align::Center,
        style(shadow, alpha),
    );
    TEXT.draw(p, text, 0.0, y, scale, Align::Center, style(colour, alpha));
}

/// An icon then a word, centred, top at `y`.
fn draw_hint<T: Target, const H: usize>(p: &mut Painter<T>, icon: &Glyph<H>, word: &str, y: f32, alpha: f32) {
    let w = icon.width as usize + 2 + TEXT.measure(word);
    let x = -floorf(w as f32 / 2.0);
    draw_glyph(p, icon, x, y + 1.0, 1.0, style(pal::DIM, alpha));
    TEXT.draw(
        p,
        word,
        x + (icon.width + 2) as f32,
        y,
        1.0,
        Align::Left,
        style(pal::WHITE, alpha),
    );
}

/// Hot Potato at rest: a calm potato with its fuse out, its name, and that
/// a shake lights it.
pub fn draw_potato_label<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
    in_pixels(c, |p| {
        let calm = Spud {
            skin: pal::POTATO,
            face: &spr::FACE_CALM,
        };
        draw_potato(p, -16.0, -21.0, &calm, None, alpha);
        draw_arcade(p, "Hot Potato", 9.0, 1.0, pal::WHITE, pal::POTATO_DARK, alpha);
        draw_hint(p, &spr::SHAKE, "light", 20.0, alpha);
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
        let (x, y) = (-16.0 + dx, -19.0 + dy);
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
        // PASS IT! flashes white and gold, faster as it heats.
        let flash = frac(t * (1.5 + 3.5 * heat)) < 0.5;
        let colour = if flash { pal::WHITE } else { pal::GOLD };
        draw_arcade(p, "PASS IT!", 10.0, 1.0, colour, pal::RED, 1.0);
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
            p.fill_rect(bx, 22.0, 5.0, 3.0, style(colour, 1.0));
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
            draw_potato(p, -16.0, -24.0, &burnt, None, left);
            for k in 0..3 {
                let u = frac(t * 0.6 + k as f32 / 3.0);
                let sx = roundf(-6.0 + 6.0 * k as f32 + sinf(u * 6.0 + k as f32) * 2.0);
                let sy = roundf(-26.0 - 8.0 * u);
                p.fill_rect(sx, sy, 2.0, 2.0, style(pal::DIM, left * (1.0 - u) * 0.7));
            }
        }
        // BOOM drops in from the top and bounces to rest.
        if t > 0.2 {
            let u = ((t - 0.2) / 0.5).min(1.0);
            let bounce = if u < 0.6 {
                let k = u / 0.6;
                -40.0 * (1.0 - k * k)
            } else {
                let k = (u - 0.6) / 0.4;
                -6.0 * sinf(k * core::f32::consts::PI)
            };
            draw_arcade(p, "BOOM", roundf(6.0 + bounce), 2.0, pal::RED, pal::GOLD, fade);
        }
        if t > 1.2 {
            let a = ((t - 1.2) / 0.4).min(1.0) * fade;
            draw_hint(p, &spr::TAP, "reset", 23.0, a);
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

/// Pass the Pot between rolls: three bills with the ones you roll lit, how
/// many that is, and that a tap changes it.
pub fn draw_bills<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, n: u8, alpha: f32) {
    in_pixels(c, |p| {
        let max = smokebomb_shared::types::MAX_POT_DICE;
        let (w, gap) = (spr::BILL.width as f32, 3.0);
        let x0 = -floorf((max as f32 * w + (max as f32 - 1.0) * gap) / 2.0);
        for i in 0..max {
            let colour = if i < n as usize { pal::MINT } else { pal::FAINT };
            draw_glyph(
                p,
                &spr::BILL,
                x0 + i as f32 * (w + gap),
                -27.0,
                1.0,
                style(colour, alpha),
            );
        }
        // "3 bills": the count in the result numerals, the word beside it on
        // their baseline.
        let mut digits: String<2> = String::new();
        let _ = write!(digits, "{n}");
        let word = if n == 1 { "bill" } else { "bills" };
        let wd = NUM_M.measure(&digits);
        let total = wd + 3 + TEXT.measure(word);
        let x = -floorf(total as f32 / 2.0);
        NUM_M.draw(
            p,
            &digits,
            x + floorf(wd as f32 / 2.0),
            -14.0,
            style(pal::WHITE, alpha),
        );
        TEXT.draw(
            p,
            word,
            x + (wd + 3) as f32,
            9.0,
            1.0,
            Align::Left,
            style(pal::WHITE, alpha),
        );
        let w = spr::TAP.width as usize + 2 + TEXT.measure("change");
        let x = -floorf(w as f32 / 2.0);
        draw_glyph(p, &spr::TAP, x, 21.0, 1.0, style(pal::DIM, alpha));
        TEXT.draw(
            p,
            "change",
            x + (spr::TAP.width + 2) as f32,
            21.0,
            1.0,
            Align::Left,
            style(pal::WHITE, alpha),
        );
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

/// A Pass the Pot die's sprite, 13 wide at scale 1, centred on `(cx, cy)`.
fn draw_pot_face<T: Target>(
    p: &mut Painter<T>,
    v: u8,
    cx: f32,
    cy: f32,
    scale: f32,
    colour: Color,
    alpha: f32,
) {
    match PotFace::from_raw(v) {
        PotFace::Left => draw_centered(p, &spr::PASS_LEFT, cx, cy, scale, style(colour, alpha)),
        PotFace::Right => draw_centered(p, &spr::PASS_RIGHT, cx, cy, scale, style(colour, alpha)),
        PotFace::Pot => {
            draw_centered(p, &spr::POT_INSIDE, cx, cy, scale, style(colour, alpha * 0.3));
            draw_centered(p, &spr::POT_RIM, cx, cy, scale, style(colour, alpha));
        }
        PotFace::Keep => draw_centered(p, &spr::KEEP, cx, cy, scale, style(colour, alpha)),
    }
}

/// The lines under a Pass the Pot result: one per kind that came up, in
/// its colour ("2 left", "1 pot"), or "keep" / "keep all".
pub fn pot_lines(values: &[u8]) -> Vec<(String<12>, Color), 3> {
    let mut counts = [0u8; 3];
    for &v in values {
        match PotFace::from_raw(v) {
            PotFace::Left => counts[0] += 1,
            PotFace::Right => counts[1] += 1,
            PotFace::Pot => counts[2] += 1,
            PotFace::Keep => {}
        }
    }
    let mut lines = Vec::new();
    for (n, (word, colour)) in
        counts
            .iter()
            .zip([("left", pal::WHITE), ("right", pal::WHITE), ("pot", pal::GOLD)])
    {
        if *n > 0 {
            let mut s = String::new();
            let _ = write!(s, "{n} {word}");
            let _ = lines.push((s, colour));
        }
    }
    if lines.is_empty() {
        let mut s = String::new();
        let _ = s.push_str(if values.len() > 1 { "keep all" } else { "keep" });
        let _ = lines.push((s, pal::MINT));
    }
    lines
}

/// A Pass the Pot result: the dice as sprites (bigger the fewer there
/// are), and under them what to do, a line per kind. A dud greys it all.
fn draw_pot_result<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, record: &RollRecord, dud: bool, alpha: f32) {
    let values = record.values.as_slice();
    let n = values.len().max(1);
    let scale = match n {
        1 => 3.0,
        2 => 2.0,
        _ => 1.0,
    };
    let lines = pot_lines(values);
    let glyph_h = 9.0 * scale;
    let block = glyph_h + 6.0 + lines.len() as f32 * LINE_PITCH - 2.0;
    let top = -floorf(block / 2.0) - 2.0;
    in_pixels(c, |p| {
        let pitch = 13.0 * scale + 6.0;
        let cy = top + floorf(glyph_h / 2.0);
        for (i, &v) in values.iter().enumerate() {
            let x = roundf((i as f32 - (n as f32 - 1.0) / 2.0) * pitch);
            let colour = if dud { pal::DIM } else { pot_face(v).1 };
            draw_pot_face(p, v, x, cy, scale, colour, alpha);
        }
        let mut y = top + glyph_h + 6.0;
        for (line, colour) in &lines {
            let colour = if dud { pal::DIM } else { *colour };
            TEXT.draw(p, line, 0.0, y, 1.0, Align::Center, style(colour, alpha));
            y += LINE_PITCH;
        }
    });
}

// ---------- the hold ring ----------

/// The hold ring: a rounded square two pixels wide, set in from the panel's
/// edge and round its corners with the glass's own radius, so it sits
/// evenly inside the lit area. It fills clockwise from 12 o'clock as a hold
/// progresses (`p`, 0–1), in whole pixels: a 96×96 ring's anti-aliased
/// line goes soft at this size. `grow` (canvas units) pushes it outward as
/// it flashes away.
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
