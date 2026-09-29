//! Screen content, ported from the mockup's drawing code (SIM_SPEC C1, C2,
//! C6). Coordinates are mockup canvas units relative to the face centre;
//! times are seconds.

use core::fmt::Write as _;

use heapless::{String, Vec};
use libm::{cosf, floorf, powf, roundf, sinf, sqrtf};
use smokebomb_hal::AssetStore;
use smokebomb_shared::{DieKind, PotFace, RollRecord};

use crate::display::{DUD, FG};
use crate::font::{fit_px, Align, Fonts};
use crate::gfx::{Painter, Style};
use crate::menu::{Draft, Page, Setup};
use crate::nest::{ChargeView, ClockView, Label, NestFace, Screen};
use crate::pigs::{throw_label, Outcome, Throw};
use crate::smoke::Special;

/// The lit area's half-size in canvas units.
pub const ACTIVE: f32 = 83.0;

/// Everything a screen needs to draw one face.
pub struct Ctx<'a, 'p, A: AssetStore> {
    pub painter: &'a mut Painter<'p>,
    pub fonts: &'a mut Fonts,
    pub assets: &'a mut A,
}

impl<A: AssetStore> Ctx<'_, '_, A> {
    fn text(&mut self, text: &str, x: f32, y: f32, px: u16, style: Style) {
        self.fonts
            .draw(self.assets, self.painter, text, x, y, px, Align::Center, style);
    }

    fn text_left(&mut self, text: &str, x: f32, y: f32, px: u16, style: Style) {
        self.fonts
            .draw(self.assets, self.painter, text, x, y, px, Align::Left, style);
    }

    /// Draw with the content moved by `(ox, oy)` canvas units (before the
    /// face's rotation) and scaled about its centre, like the mockup's
    /// `translate(S/2 + ox, S/2 + oy); rotate(angle); scale(s)`.
    fn shifted(&mut self, ox: f32, oy: f32, scale: f32, draw: impl FnOnce(&mut Self)) {
        let base = self.painter.xf;
        self.painter.xf = base.offset(ox, oy).scaled(scale);
        draw(self);
        self.painter.xf = base;
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

// ---------- boot (C1) ----------

/// Pip slots, in units of the pip grid offset.
const PIP_TL: (f32, f32) = (-1.0, -1.0);
const PIP_BR: (f32, f32) = (1.0, 1.0);
const PIP_TR: (f32, f32) = (1.0, -1.0);
const PIP_BL: (f32, f32) = (-1.0, 1.0);
const PIP_C: (f32, f32) = (0.0, 0.0);
const PIP_ML: (f32, f32) = (-1.0, 0.0);
const PIP_MR: (f32, f32) = (1.0, 0.0);

/// Slot order per value: each step slides existing pips and grows or
/// shrinks the rest.
fn pips(value: u8) -> &'static [(f32, f32)] {
    match value {
        1 => &[PIP_C],
        2 => &[PIP_TL, PIP_BR],
        3 => &[PIP_TL, PIP_BR, PIP_C],
        4 => &[PIP_TL, PIP_BR, PIP_TR, PIP_BL],
        5 => &[PIP_TL, PIP_BR, PIP_TR, PIP_BL, PIP_C],
        _ => &[PIP_TL, PIP_BR, PIP_TR, PIP_BL, PIP_ML, PIP_MR],
    }
}

/// Start value per face index (opposite faces sum to 7).
const FACE_START: [u8; 6] = [3, 4, 1, 6, 2, 5];
const BOOT_STEP: f32 = 0.34;
const BOOT_STEPS: u32 = 5;
const BOOT_FADE: f32 = 0.4;
pub const LOOP_END: f32 = 0.25 + BOOT_STEP * BOOT_STEPS as f32;
/// Whole boot sequence, seconds.
pub const BOOT_DURATION: f32 = LOOP_END + 4.25;
/// The lone centre pip waits this long before it bursts.
pub const BURST_AT: f32 = 0.75;
const PIP_GRID: f32 = ACTIVE * 0.46;
const PIP_RADIUS: f32 = ACTIVE * 0.15;
const PIP_GLOW: f32 = 10.0;

fn draw_pips<A: AssetStore>(c: &mut Ctx<A>, from: u8, to: u8, t: f32, alpha: f32, scale: f32) {
    let (a, b) = (pips(from), pips(to));
    let e = smoothstep(t);
    for i in 0..a.len().max(b.len()) {
        let pa = a.get(i).copied().unwrap_or(PIP_C);
        let pb = b.get(i).copied().unwrap_or(PIP_C);
        let sa = if i < a.len() { 1.0 } else { 0.0 };
        let sb = if i < b.len() { 1.0 } else { 0.0 };
        let sc = (sa + (sb - sa) * e) * scale;
        if sc <= 0.01 {
            continue;
        }
        let x = (pa.0 + (pb.0 - pa.0) * e) * PIP_GRID;
        let y = (pa.1 + (pb.1 - pa.1) * e) * PIP_GRID;
        let style = Style::new(FG, alpha * (sc * 1.5).min(1.0), PIP_GLOW);
        c.painter.fill_circle(x, y, PIP_RADIUS * sc, style);
    }
}

/// One face of the boot animation. `t` is seconds since boot, `index` the
/// face index, `top` whether this face was on top when boot started.
pub fn draw_boot<A: AssetStore>(c: &mut Ctx<A>, index: usize, top: bool, t: f32) {
    // A slight stagger around the cube.
    let t = t - if top { 0.0 } else { index as f32 * 0.05 };
    if t < 0.0 {
        return;
    }
    if top && t >= LOOP_END {
        draw_boot_finale(c, t - LOOP_END);
        return;
    }
    // The top face starts on 6 so the loop lands it on 5.
    let v0 = if top { 6 } else { FACE_START[index] };
    let val = |k: u32| ((v0 as u32 - 1 + k) % 6 + 1) as u8;
    let appear = (t / 0.25).min(1.0);
    let steps_t = t - 0.25;
    let fade = ((steps_t - BOOT_STEP * BOOT_STEPS as f32) / BOOT_FADE).clamp(0.0, 1.0);
    if steps_t < 0.0 {
        draw_pips(c, v0, v0, 0.0, appear, appear);
        return;
    }
    let k = (floorf(steps_t / BOOT_STEP) as u32).min(BOOT_STEPS);
    let local = if k >= BOOT_STEPS {
        1.0
    } else {
        ((steps_t - k as f32 * BOOT_STEP) / 0.22).min(1.0)
    };
    let from = val(k.min(BOOT_STEPS - 1));
    let to = val((k + 1).min(BOOT_STEPS));
    let from = if k >= BOOT_STEPS { to } else { from };
    draw_pips(c, from, to, local, 1.0 - fade, 1.0 - 0.6 * fade);
}

/// Top face after the loop: corner pips shoot outward, the centre pip
/// breathes and bursts, and the name emerges.
fn draw_boot_finale<A: AssetStore>(c: &mut Ctx<A>, u: f32) {
    if u < 0.35 {
        let e = powf(u / 0.35, 2.0);
        let d = PIP_GRID * (1.0 + 1.4 * e);
        let style = Style::new(FG, 1.0 - e, PIP_GLOW);
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            c.painter
                .fill_circle(x * d, y * d, PIP_RADIUS * (1.0 - 0.4 * e), style);
        }
    }
    if u < BURST_AT + 0.15 {
        let b = ((u - BURST_AT) / 0.15).max(0.0);
        let sc = if u < BURST_AT {
            1.0 + 0.07 * sinf(u * core::f32::consts::PI * 4.0)
        } else {
            1.0 + b * 1.2
        };
        c.painter
            .fill_circle(0.0, 0.0, PIP_RADIUS * sc, Style::new(FG, 1.0 - b, PIP_GLOW));
    }
    let la = ((u - 1.65) / 0.6).clamp(0.0, 1.0)
        * if u < 3.8 {
            1.0
        } else {
            (1.0 - (u - 3.8) / 0.4).max(0.0)
        };
    if la > 0.0 {
        let px = fit_px("SMOKEBOMB", 28, 142.0);
        c.text("SMOKEBOMB", 0.0, 2.0, px, Style::new(FG, la, 14.0));
    }
}

// ---------- setup label (C2) ----------

/// The setup as the die shows it: `d20`, `3d6`, `Pass the Pot`, `Pass the Pot ×2`.
pub fn setup_label(die: DieKind, count: u8) -> String<24> {
    let mut s = String::new();
    let _ = match (die, count) {
        (DieKind::PassThePot, 1) => write!(s, "Pass the Pot"),
        (DieKind::PassThePot, n) => write!(s, "Pass the Pot ×{n}"),
        (d, 1) => write!(s, "{}", d.wire_name()),
        (d, n) => write!(s, "{n}{}", d.wire_name()),
    };
    s
}

/// The wake/setup label: the setup's icon (a die's solid, a bomb, a banknote)
/// above its name, at `alpha` (already including the 85%).
pub fn draw_wake_label<A: AssetStore>(c: &mut Ctx<A>, setup: Setup, label: &str, alpha: f32) {
    crate::icons::draw_setup_icon(c, setup, 0.0, -21.0, 36.0, alpha);
    c.text(
        label,
        0.0,
        44.0,
        fit_px(label, 28, 150.0),
        Style::new(FG, alpha, 8.0),
    );
}

// ---------- results (C6) ----------

/// Big number size by character count (canvas px).
fn big_size(chars: usize) -> f32 {
    match chars {
        0..=2 => 94.0,
        3 => 66.0,
        4 => 52.0,
        5 | 6 => 40.0,
        _ => 32.0,
    }
}

/// A roll's result: every face except the one facing down shows this.
/// A max says so in its label; a dud greys out and says "dud" (C6).
pub fn draw_result<A: AssetStore>(c: &mut Ctx<A>, record: &RollRecord, special: Option<Special>, alpha: f32) {
    let setup = setup_label(record.die, record.values.len() as u8);
    let mut label: String<32> = String::new();
    let _ = match special {
        Some(Special::Max) => write!(label, "max {setup}"),
        Some(Special::Dud) => write!(label, "dud"),
        None => write!(label, "{setup}"),
    };
    let value = if special == Some(Special::Dud) { DUD } else { FG };
    if !record.die.is_numeric() {
        draw_pot_tokens(c, &record.values, value, alpha);
        c.text(
            &label,
            0.0,
            54.0,
            fit_px(&label, 22, 150.0),
            Style::new(value, alpha, 8.0),
        );
        return;
    }

    let mut big: String<8> = String::new();
    let _ = write!(big, "{}", record.total());
    let mut parts: String<48> = String::new();
    if record.values.len() > 1 {
        for (i, v) in record.values.iter().enumerate() {
            let _ = write!(parts, "{}{v}", if i > 0 { "+" } else { "" });
        }
        if parts.len() > 18 {
            parts.clear();
        }
    }
    let has_parts = !parts.is_empty();
    let px = roundf(big_size(big.len()) * if has_parts { 0.85 } else { 1.0 }) as u16;
    c.text(
        &big,
        0.0,
        if has_parts { 0.0 } else { -12.0 },
        px,
        Style::new(value, alpha, 18.0),
    );
    c.text(
        &label,
        0.0,
        if has_parts { 56.0 } else { 54.0 },
        fit_px(&label, 22, 150.0),
        Style::new(value, alpha, 8.0),
    );
    if has_parts {
        let px = if parts.len() > 10 { 14 } else { 17 };
        c.text(&parts, 0.0, -60.0, px, Style::new(value, alpha * 0.7, 8.0));
    }
}

/// Pass the Pot glyphs in a row: arrows pass left or right, the pot glyph
/// feeds the pot, a dot keeps.
fn draw_pot_tokens<A: AssetStore>(c: &mut Ctx<A>, values: &[u8], value: u8, alpha: f32) {
    let n = values.len();
    let sz = match n {
        1 => 64.0,
        2 => 50.0,
        _ => 40.0,
    };
    let gap = sz * 1.15;
    let y = -12.0;
    let style = Style::new(value, alpha, 14.0);
    for (i, &v) in values.iter().enumerate() {
        let x = (i as f32 - (n as f32 - 1.0) / 2.0) * gap;
        match PotFace::from_raw(v) {
            PotFace::Keep => c.painter.fill_circle(x, y, sz * 0.13, style),
            PotFace::Pot => {
                let w = sz * 0.8;
                let rim = y - sz * 0.05;
                c.painter
                    .stroke_polyline(&[(x - w * 0.55, rim), (x + w * 0.55, rim)], sz * 0.09, style);
                c.painter
                    .stroke_arc(x, rim, w * 0.45, 0.0, core::f32::consts::PI, sz * 0.09, style);
                c.painter.fill_circle(x, y - sz * 0.32, sz * 0.11, style);
            }
            dir @ (PotFace::Left | PotFace::Right) => {
                let d = if dir == PotFace::Right { 1.0 } else { -1.0 };
                let (l, h) = (sz * 0.36, sz * 0.2);
                let shaft = [(x - d * l, y), (x + d * l, y)];
                let head = [(x + d * (l - h), y - h), (x + d * l, y), (x + d * (l - h), y + h)];
                c.painter.stroke_paths(&[&shaft, &head], sz * 0.1, style);
            }
        }
    }
}

// ---------- Pig Toss ----------

/// When each part of the score sequence starts, in seconds after the die
/// lands. The pigs settle first ([`crate::pigfx::SETTLE_S`]).
pub mod pig_score {
    /// The pigs move up and shrink.
    pub const SHRINK_AT: f32 = 0.9;
    pub const SHRINK_S: f32 = 0.4;
    /// The throw's points pop in.
    pub const POP_AT: f32 = 1.0;
    pub const LABEL_AT: f32 = 1.3;
    /// The turn's total counts up.
    pub const TURN_AT: f32 = 1.8;
    pub const COUNT_S: f32 = 0.8;
    /// Whose turn it is, then the prompt in its place.
    pub const WHO_AT: f32 = 2.6;
    pub const PROMPT_AT: f32 = 3.4;
}

/// 0 before `at`, rising to 1 over `over` seconds.
fn ramp(t: f32, at: f32, over: f32) -> f32 {
    ((t - at) / over).clamp(0.0, 1.0)
}

/// What a throw says once the pigs have settled, `t` seconds after the die
/// landed: the throw's points pop in big, the turn's points count up beside
/// the player's total, and then the screen says what to do next. The pigs themselves are
/// [`crate::pigfx`]. A bust dims to the dud colour and passes the die by
/// itself, so it only asks for the next player to shake.
pub fn draw_pig_score<A: AssetStore>(
    c: &mut Ctx<A>,
    throw: &Throw,
    t: f32,
    banked: u16,
    next_player: u8,
    alpha: f32,
) {
    use pig_score::*;
    let bust = throw.outcome == Outcome::Bust;
    let value = if bust { DUD } else { FG };
    let style = |a: f32, glow: f32| Style::new(value, alpha * a, glow);

    // The big number, popping in and settling.
    let pop = ramp(t, POP_AT, 0.35);
    if pop > 0.0 {
        let mut big: String<8> = String::new();
        let _ = match throw.outcome {
            Outcome::Bust => write!(big, "BUST"),
            Outcome::Score(p) => write!(big, "+{p}"),
        };
        let px = big_size(big.len()).min(52.0) * 0.8;
        let over = 1.0 - pop;
        c.text(
            &big,
            0.0,
            -12.0,
            roundf(px * (1.0 + 0.7 * over * over)) as u16,
            style((pop * 3.0).min(1.0), 14.0),
        );
    }

    let label_a = ramp(t, LABEL_AT, 0.25);
    if label_a > 0.0 {
        let mut label: String<32> = String::new();
        let _ = match throw.outcome {
            Outcome::Bust if throw.turn_before > 0 => write!(label, "Lost {}", throw.turn_before),
            Outcome::Bust => write!(label, "Nothing to lose"),
            Outcome::Score(_) => write!(label, "{}", throw_label(throw.poses)),
        };
        c.text(
            &label,
            0.0,
            13.0,
            fit_px(&label, 17, 150.0),
            style(label_a * 0.85, 6.0),
        );
    }

    let turn_a = ramp(t, TURN_AT, 0.25);
    if turn_a > 0.0 {
        let mut line: String<24> = String::new();
        let _ = match throw.outcome {
            Outcome::Bust => write!(line, "Pass to P{}", next_player + 1),
            Outcome::Score(p) => {
                // Counts up from what the turn was to what it is now.
                let k = ramp(t, TURN_AT, COUNT_S);
                let eased = 1.0 - (1.0 - k) * (1.0 - k);
                let turn = throw.turn_before + roundf(p as f32 * eased) as u16;
                // The turn's points, then what the player would have in all.
                write!(line, "{turn} · {}", banked + turn)
            }
        };
        c.text(&line, 0.0, 36.0, fit_px(&line, 24, 150.0), style(turn_a, 10.0));
    }

    // The bottom row: whose turn it is, then what to do next.
    let prompt = ramp(t, PROMPT_AT, 0.3);
    let who_a = ramp(t, WHO_AT, 0.25) * (1.0 - prompt);
    if who_a > 0.0 && !bust {
        let mut who: String<24> = String::new();
        let _ = write!(who, "P{}", throw.player + 1);
        c.text(&who, 0.0, 66.0, 14, style(who_a * 0.7, 0.0));
    }
    if prompt > 0.0 {
        if bust {
            let mut line: String<24> = String::new();
            let _ = write!(line, "P{}: shake to roll", next_player + 1);
            c.text(
                &line,
                0.0,
                66.0,
                fit_px(&line, 15, 150.0),
                style(prompt * 0.9, 6.0),
            );
        } else {
            c.text("Shake to roll again", 0.0, 56.0, 14, style(prompt * 0.7, 0.0));
            // The action that matters breathes a little.
            let breathe = 0.85 + 0.15 * sinf((t - PROMPT_AT) * 4.0);
            let line = "Tap top to bank & pass";
            c.text(
                line,
                0.0,
                72.0,
                fit_px(line, 15, 150.0),
                style(prompt * breathe, 6.0),
            );
        }
    }
}

// ---------- Hot Potato ----------

/// A lit fuse: a glow that swells with `heat` (0–1) and flashes on each
/// tick (`pulse`, 0–1), with a nudge underneath.
pub fn draw_fuse<A: AssetStore>(c: &mut Ctx<A>, heat: f32, pulse: f32) {
    let core = 16.0 + 24.0 * heat;
    c.painter.fill_circle(
        0.0,
        -6.0,
        core,
        Style::new(FG, 0.30 + 0.45 * heat, 10.0 + 14.0 * heat),
    );
    c.painter
        .fill_circle(0.0, -6.0, core * 0.55, Style::new(FG, 0.55 + 0.45 * pulse, 12.0));
    if pulse > 0.0 {
        c.painter.fill_circle(
            0.0,
            -6.0,
            core + 14.0 * (1.0 - pulse),
            Style::new(FG, 0.35 * pulse, 0.0),
        );
    }
    c.text("PASS IT", 0.0, 62.0, 14, Style::new(FG, 0.7, 0.0));
}

/// The fuse ran out: a shockwave, then BOOM, and a nudge to reset. `t` is
/// seconds since it went off.
pub fn draw_boom<A: AssetStore>(c: &mut Ctx<A>, t: f32) {
    let p = (t / 0.7).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - p) * (1.0 - p) * (1.0 - p);
    if p < 1.0 {
        c.painter
            .fill_circle(0.0, 0.0, 8.0 + 92.0 * e, Style::new(FG, 0.9 * (1.0 - p), 20.0));
    }
    let fade = (1.0 - (t - 4.6) / 1.0).clamp(0.0, 1.0);
    let a = (t / 0.12).min(1.0) * fade;
    c.text(
        "BOOM",
        0.0,
        -4.0,
        fit_px("BOOM", 44, 150.0),
        Style::new(FG, a, 16.0),
    );
    if t > 1.2 {
        let a = ((t - 1.2) / 0.4).min(1.0) * fade;
        c.text("Tap to reset", 0.0, 52.0, 13, Style::new(FG, 0.7 * a, 0.0));
    }
}

// ---------- menu (C3, C4) ----------

/// How far menu content slides during a tip, canvas units.
pub const TIP_SLIDE: f32 = 120.0;

/// One menu page: status bar (setup and battery), title, ▲/▼, the value,
/// and page dots. `battery` is 0–1.
#[allow(clippy::too_many_arguments)]
pub fn draw_menu<A: AssetStore>(
    c: &mut Ctx<A>,
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
    c.shifted(ox, oy, scale, |c| {
        let status = Style::new(FG, 0.85 * alpha, 0.0);
        c.text_left(&m.setup().short_label(), -66.0, -66.0, 14, status);
        c.painter.stroke_rect(44.0, -71.0, 20.0, 10.0, 1.5, status);
        c.painter.fill_rect(64.0, -68.0, 2.0, 4.0, status);
        c.painter
            .fill_rect(46.0, -69.0, 16.0 * battery.clamp(0.0, 1.0), 6.0, status);

        c.text(m.page.title(), 0.0, -40.0, 15, Style::new(FG, 0.75 * alpha, 0.0));
        let arrows = Style::new(FG, 0.55 * alpha, 0.0);
        let (w, h) = (4.5, 4.0);
        c.painter
            .fill_triangle([(0.0, -22.0 - h), (w, -22.0 + h), (-w, -22.0 + h)], arrows);
        c.painter
            .fill_triangle([(-w, 46.0 - h), (w, 46.0 - h), (0.0, 46.0 + h)], arrows);

        if m.page == Page::Settings {
            let name = m.setting().0;
            let value = m.setting_value();
            c.text(
                name,
                0.0,
                2.0,
                fit_px(name, 26, 150.0),
                Style::new(FG, alpha, 10.0),
            );
            c.text(
                &value,
                0.0,
                27.0,
                fit_px(&value, 18, 150.0),
                Style::new(FG, 0.72 * alpha, 0.0),
            );
            if let Some(detail) = m.setting_detail() {
                c.text(
                    detail,
                    0.0,
                    46.0,
                    fit_px(detail, 18, 150.0),
                    Style::new(FG, 0.72 * alpha, 0.0),
                );
            }
        } else {
            let value = m.value();
            c.text(
                &value,
                0.0,
                12.0,
                fit_px(&value, 52, 150.0),
                Style::new(FG, alpha, 14.0),
            );
        }

        let n = m.ring().len();
        for i in 0..n {
            let x = (i as f32 - (n as f32 - 1.0) / 2.0) * 14.0;
            let a = if i == m.page_index() { 1.0 } else { 0.35 };
            c.painter
                .fill_circle(x, 66.0, 3.5, Style::new(FG, a * alpha, 0.0));
        }
    });
}

/// The hold ring: a rounded square just inside the lit area that fills
/// clockwise from 12 o'clock as a hold progresses (`p`, 0–1). `grow` pushes
/// it outward as it flashes away.
pub fn draw_hold_ring<A: AssetStore>(c: &mut Ctx<A>, p: f32, alpha: f32, grow: f32) {
    if alpha <= 0.0 || p <= 0.0 {
        return;
    }
    let inset = 3.0 - grow;
    let (x0, x1) = (-ACTIVE + inset, ACTIVE - inset);
    let (y0, y1) = (x0, x1);
    let r = 24.0 - inset;
    // The mockup's dash length assumes circular corners.
    let total = 4.0 * (x1 - x0 - 2.0 * r) + 2.0 * core::f32::consts::PI * r;
    let mut path = Path::new(0.0, y0, p * total);
    path.line(x1 - r, y0);
    path.quad((x1, y0), (x1, y0 + r));
    path.line(x1, y1 - r);
    path.quad((x1, y1), (x1 - r, y1));
    path.line(x0 + r, y1);
    path.quad((x0, y1), (x0, y1 - r));
    path.line(x0, y0 + r);
    path.quad((x0, y0), (x0 + r, y0));
    path.line(0.0, y0);
    c.painter
        .stroke_polyline(&path.points, 4.0, Style::new(FG, alpha, 4.0));
}

/// A polyline that stops after `budget` canvas units, for dashed strokes.
struct Path {
    points: Vec<(f32, f32), 64>,
    budget: f32,
}

impl Path {
    fn new(x: f32, y: f32, budget: f32) -> Self {
        let mut points = Vec::new();
        let _ = points.push((x, y));
        Self { points, budget }
    }

    fn line(&mut self, x: f32, y: f32) {
        if self.budget <= 0.0 {
            return;
        }
        let &(px, py) = self.points.last().unwrap_or(&(x, y));
        let len = sqrtf((x - px) * (x - px) + (y - py) * (y - py));
        let (x, y) = if len > self.budget {
            let k = self.budget / len;
            (px + (x - px) * k, py + (y - py) * k)
        } else {
            (x, y)
        };
        self.budget -= len;
        let _ = self.points.push((x, y));
    }

    fn quad(&mut self, ctrl: (f32, f32), to: (f32, f32)) {
        let from = self.points.last().copied().unwrap_or(to);
        const STEPS: usize = 8;
        for i in 1..=STEPS {
            let t = i as f32 / STEPS as f32;
            let u = 1.0 - t;
            let x = u * u * from.0 + 2.0 * u * t * ctrl.0 + t * t * to.0;
            let y = u * u * from.1 + 2.0 * u * t * ctrl.1 + t * t * to.1;
            self.line(x, y);
        }
    }
}

/// After a save: a check draws itself, then the setup and a nudge to play.
/// `t` is seconds since the save.
pub fn draw_success<A: AssetStore>(c: &mut Ctx<A>, label: &str, nudge: &str, t: f32) {
    let a = (t / 0.15).min(1.0)
        * if t < 0.9 {
            1.0
        } else {
            (1.0 - (t - 0.9) / 0.35).max(0.0)
        };
    if a <= 0.0 {
        return;
    }
    let p = ((t - 0.05) / 0.3).clamp(0.0, 1.0);
    let (l1, l2) = (sqrtf(200.0), sqrtf(884.0));
    let d = p * (l1 + l2);
    let mut check: Vec<(f32, f32), 3> = Vec::new();
    let _ = check.push((-14.0, -40.0));
    if d <= l1 {
        let _ = check.push((-14.0 + 10.0 * d / l1, -40.0 + 10.0 * d / l1));
    } else {
        let k = (d - l1) / l2;
        let _ = check.push((-4.0, -30.0));
        let _ = check.push((-4.0 + 20.0 * k, -30.0 - 22.0 * k));
    }
    c.painter.stroke_polyline(&check, 5.0, Style::new(FG, a, 8.0));
    c.text(label, 0.0, 6.0, fit_px(label, 50, 150.0), Style::new(FG, a, 14.0));
    c.text(nudge, 0.0, 48.0, 15, Style::new(FG, a * 0.75, 0.0));
}

/// The whole-face flash when a setup is saved.
pub fn draw_flash<A: AssetStore>(c: &mut Ctx<A>, alpha: f32) {
    if alpha > 0.0 {
        // #e8ecf2 at 35%.
        c.painter
            .fill_rect(-128.0, -128.0, 256.0, 256.0, Style::new(0xf2, alpha * 0.35, 0.0));
    }
}

// ---------- the Nest (DOCK_BRIEF) ----------

/// The clock's radius in canvas units (C7).
const CLOCK_R: f32 = 71.0;

/// One Nest face. Dimming is the caller's: it scales the finished frame.
pub fn draw_nest<A: AssetStore>(c: &mut Ctx<A>, face: &NestFace) {
    let base = c.painter.xf;
    c.painter.xf = base.offset(face.shift.0, face.shift.1);
    match face.screen {
        Screen::Blank => {}
        Screen::Charge(v) => draw_charge(c, &v),
        Screen::Clock(v) => draw_clock(c, &v),
        Screen::Flip { t } => draw_flip(c, t),
        Screen::SideDown { pulse } => {
            let a = pulse;
            draw_chevron(c, (0.0, -1.0), (0.0, -14.0), a);
            c.text(
                "This side down",
                0.0,
                40.0,
                fit_px("This side down", 20, 150.0),
                Style::new(FG, 0.9, 6.0),
            );
        }
        Screen::TipArrow { dir, nudge } => {
            // 4 px toward the edge and back.
            let n = nudge * 4.0 / crate::gfx::K;
            draw_arrow(c, dir, (dir.0 * n, -14.0 + dir.1 * n), 1.0);
            c.text(
                "Tip this way",
                0.0,
                52.0,
                fit_px("Tip this way", 27, 150.0),
                Style::new(FG, 1.0, 8.0),
            );
        }
        Screen::FaceDown => {
            draw_arrow(c, (0.0, 1.0), (0.0, -14.0), 1.0);
            c.text(
                "This side down",
                0.0,
                52.0,
                fit_px("This side down", 27, 150.0),
                Style::new(FG, 1.0, 8.0),
            );
        }
        Screen::Toward { dir } => draw_chevron(c, dir, (0.0, 0.0), 0.5),
        Screen::NoPower => draw_no_power(c),
        Screen::Fault => draw_fault(c),
    }
    c.painter.xf = base;
}

/// The battery: a liquid fill with a wavy top edge, the percentage, and a
/// status word.
fn draw_charge<A: AssetStore>(c: &mut Ctx<A>, v: &ChargeView) {
    if v.alpha <= 0.0 {
        return;
    }
    if v.fill > 0.002 {
        let top = ACTIVE - v.fill.min(1.0) * 2.0 * ACTIVE;
        let amp = 4.0 * (v.fill * 20.0).min(1.0);
        let a = (0.28 + 0.72 * v.flash) * v.alpha;
        c.painter.shape(
            (-ACTIVE, -ACTIVE, ACTIVE, ACTIVE),
            Style::new(FG, a, 0.0),
            |x, y| {
                let edge = top + amp * sinf(0.09 * x + 2.2 * v.wave);
                (edge - y).max(y - ACTIVE)
            },
        );
    }
    let mut pct: String<8> = String::new();
    let _ = write!(pct, "{}%", roundf(v.pct) as u32);
    let ta = v.alpha * v.text;
    c.text(&pct, 0.0, -8.0, fit_px(&pct, 52, 150.0), Style::new(FG, ta, 10.0));
    let word = match v.label {
        Label::Charging => "charging",
        Label::Full => "full",
        Label::Battery => "battery",
    };
    c.text(word, 0.0, 38.0, 20, Style::new(FG, ta * 0.8, 4.0));
}

/// The analog clock (C7): ticks, then hour, minute and smooth second hands.
fn draw_clock<A: AssetStore>(c: &mut Ctx<A>, v: &ClockView) {
    if v.alpha <= 0.0 {
        return;
    }
    if v.smoke > 0.0 {
        draw_wisp(c, v.smoke);
    }
    let dir = |turn: f32| {
        let a = core::f32::consts::TAU * turn - core::f32::consts::FRAC_PI_2;
        (cosf(a), sinf(a))
    };
    if v.ticks > 0.0 {
        let mut major = [[(0.0, 0.0); 2]; 4];
        let mut minor = [[(0.0, 0.0); 2]; 8];
        let (mut mi, mut ni) = (0, 0);
        for i in 0..12 {
            let (dx, dy) = dir(i as f32 / 12.0);
            let is_major = i % 3 == 0;
            let r0 = if is_major { CLOCK_R - 12.0 } else { CLOCK_R - 7.0 };
            let seg = [(dx * r0, dy * r0), (dx * CLOCK_R, dy * CLOCK_R)];
            if is_major {
                major[mi] = seg;
                mi += 1;
            } else {
                minor[ni] = seg;
                ni += 1;
            }
        }
        let majors: [&[(f32, f32)]; 4] = core::array::from_fn(|i| &major[i][..]);
        let minors: [&[(f32, f32)]; 8] = core::array::from_fn(|i| &minor[i][..]);
        let a = v.alpha * v.ticks;
        c.painter.stroke_paths(&minors, 2.5, Style::new(FG, 0.5 * a, 0.0));
        c.painter.stroke_paths(&majors, 4.0, Style::new(FG, 0.9 * a, 0.0));
    }
    // Where each hand points, as a turn: hour 0–12 h, minute 0–1 h, second
    // 0–1 min. Docking sweeps them from 12:00; undocking spins one more turn.
    let t = v.secs;
    let turns = [(t / 43_200.0) % 1.0, (t / 3_600.0) % 1.0, (t / 60.0) % 1.0];
    let hands = [
        (0.52 * CLOCK_R, 6.0, 1.0),
        (0.8 * CLOCK_R, 4.0, 1.0),
        (0.86 * CLOCK_R, 1.8, 0.7),
    ];
    for (turn, (len, width, alpha)) in turns.into_iter().zip(hands) {
        let (dx, dy) = dir(turn * v.hands + v.spin);
        c.painter.stroke_polyline(
            &[(-dx * 6.0, -dy * 6.0), (dx * len, dy * len)],
            width,
            Style::new(FG, alpha * v.alpha, 6.0),
        );
    }
    c.painter.fill_circle(0.0, 0.0, 4.5, Style::new(FG, v.alpha, 6.0));
    if let Some(p) = v.puff {
        draw_puff(c, p);
    }
}

/// A wisp of smoke sinking toward the bottom edge as the die is seated:
/// ten soft puffs from mid-height, fading as they fall.
fn draw_wisp<A: AssetStore>(c: &mut Ctx<A>, t: f32) {
    for k in 0..10 {
        let seed = (k * 37 + 11) % 100;
        let x = (seed as f32 - 50.0) * 1.1;
        let delay = (k % 5) as f32 * 0.08;
        let u = ((t - delay) / (1.0 - 0.4)).clamp(0.0, 1.0);
        let y = u * (ACTIVE - 4.0);
        let r = 7.0 + (k % 3) as f32 * 3.0 + 6.0 * u;
        let a = 0.45 * sinf(core::f32::consts::PI * u.min(1.0)).max(0.0);
        c.painter.fill_circle(x, y, r, Style::new(FG, a, 10.0));
    }
}

/// The small puff the clock dissolves into as the die is lifted.
fn draw_puff<A: AssetStore>(c: &mut Ctx<A>, p: f32) {
    for k in 0..7 {
        let a = core::f32::consts::TAU * k as f32 / 7.0 + 0.4;
        let d = 8.0 + 26.0 * p;
        let r = 9.0 + 10.0 * p;
        c.painter
            .fill_circle(cosf(a) * d, sinf(a) * d, r, Style::new(FG, 0.5 * (1.0 - p), 10.0));
    }
}

/// A chevron pointing along `dir`, centred on `at`.
fn draw_chevron<A: AssetStore>(c: &mut Ctx<A>, dir: (f32, f32), at: (f32, f32), alpha: f32) {
    let perp = (-dir.1, dir.0);
    let tip = (at.0 + dir.0 * 14.0, at.1 + dir.1 * 14.0);
    let arm = |s: f32| {
        (
            tip.0 - dir.0 * 28.0 + perp.0 * 30.0 * s,
            tip.1 - dir.1 * 28.0 + perp.1 * 30.0 * s,
        )
    };
    c.painter
        .stroke_polyline(&[arm(1.0), tip, arm(-1.0)], 12.0, Style::new(FG, alpha, 8.0));
}

/// A large arrow along `dir`, centred on `at`.
fn draw_arrow<A: AssetStore>(c: &mut Ctx<A>, dir: (f32, f32), at: (f32, f32), alpha: f32) {
    let perp = (-dir.1, dir.0);
    let at_pt = |along: f32, side: f32| {
        (
            at.0 + dir.0 * along + perp.0 * side,
            at.1 + dir.1 * along + perp.1 * side,
        )
    };
    let shaft = [at_pt(-30.0, 0.0), at_pt(30.0, 0.0)];
    let head = [at_pt(12.0, 20.0), at_pt(32.0, 0.0), at_pt(12.0, -20.0)];
    c.painter
        .stroke_paths(&[&shaft, &head], 10.0, Style::new(FG, alpha, 8.0));
}

/// "Flip me over": a square turning over, under a curved arrow.
fn draw_flip<A: AssetStore>(c: &mut Ctx<A>, t: f32) {
    let u = (t / 1.2) % 1.0;
    let turn = ease_in_out(u) * core::f32::consts::PI;
    let h = (40.0 * cosf(turn)).abs().max(3.0);
    let style = Style::new(FG, 1.0, 8.0);
    c.painter.stroke_rect(-20.0, -6.0 - h / 2.0, 40.0, h, 5.0, style);
    // The arrow arcs over the top and points down the far side.
    let (r, end) = (36.0, -0.45);
    c.painter.stroke_arc(0.0, -6.0, r, -2.7, end, 5.0, style);
    let p = (r * cosf(end), -6.0 + r * sinf(end));
    let tan = (-sinf(end), cosf(end));
    let n = (tan.1, -tan.0);
    c.painter.fill_triangle(
        [
            (p.0 + tan.0 * 12.0, p.1 + tan.1 * 12.0),
            (p.0 + n.0 * 9.0, p.1 + n.1 * 9.0),
            (p.0 - n.0 * 9.0, p.1 - n.1 * 9.0),
        ],
        style,
    );
    c.text(
        "Flip me over",
        0.0,
        52.0,
        fit_px("Flip me over", 27, 150.0),
        Style::new(FG, 1.0, 8.0),
    );
}

fn ease_in_out(u: f32) -> f32 {
    u * u * (3.0 - 2.0 * u)
}

/// A plug, and "Check the Nest / cable or contacts".
fn draw_no_power<A: AssetStore>(c: &mut Ctx<A>) {
    let style = Style::new(FG, 1.0, 8.0);
    c.painter.stroke_rect(-16.0, -32.0, 32.0, 24.0, 5.0, style);
    c.painter.fill_rect(-10.0, -50.0, 5.0, 14.0, style);
    c.painter.fill_rect(5.0, -50.0, 5.0, 14.0, style);
    c.painter
        .stroke_polyline(&[(0.0, -8.0), (0.0, 2.0), (12.0, 12.0)], 5.0, style);
    c.text(
        "Check the Nest",
        0.0,
        28.0,
        fit_px("Check the Nest", 27, 150.0),
        style,
    );
    c.text("cable or contacts", 0.0, 56.0, 15, Style::new(FG, 0.7, 0.0));
}

/// A warning triangle, and "Charging paused".
fn draw_fault<A: AssetStore>(c: &mut Ctx<A>) {
    let style = Style::new(FG, 1.0, 8.0);
    c.painter.stroke_polyline(
        &[(0.0, -48.0), (32.0, 6.0), (-32.0, 6.0), (0.0, -48.0)],
        5.0,
        style,
    );
    c.painter
        .stroke_polyline(&[(0.0, -32.0), (0.0, -16.0)], 5.0, style);
    c.painter.fill_circle(0.0, -4.0, 3.0, style);
    c.text(
        "Charging paused",
        0.0,
        38.0,
        fit_px("Charging paused", 24, 150.0),
        style,
    );
}

/// The low-battery glyph: a small lightning bolt at the centre.
pub fn draw_bolt<A: AssetStore>(c: &mut Ctx<A>, alpha: f32) {
    c.painter.stroke_polyline(
        &[(4.0, -12.0), (-5.0, 1.0), (5.0, -1.0), (-4.0, 12.0)],
        4.0,
        Style::new(FG, alpha, 8.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_labels_match_the_mockup() {
        assert_eq!(setup_label(DieKind::D20, 1).as_str(), "d20");
        assert_eq!(setup_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(setup_label(DieKind::PassThePot, 1).as_str(), "Pass the Pot");
        assert_eq!(setup_label(DieKind::PassThePot, 2).as_str(), "Pass the Pot ×2");
    }

    #[test]
    fn boot_timing_matches_the_spec() {
        assert!((LOOP_END - 1.95).abs() < 1e-6);
        assert!((BOOT_DURATION - 6.2).abs() < 1e-6);
    }
}
