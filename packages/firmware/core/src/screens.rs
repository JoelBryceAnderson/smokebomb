//! Screen content, ported from the mockup's drawing code (SIM_SPEC C1, C2,
//! C6). Coordinates are mockup canvas units relative to the face centre;
//! times are seconds.

use core::fmt::Write as _;

use heapless::{String, Vec};
use libm::{cosf, fabsf, floorf, powf, roundf, sinf, sqrtf};
use smokebomb_hal::{AssetStore, Color, Grey96, Target};
use smokebomb_shared::{DieKind, PotFace, RollRecord};

use crate::display::{DUD, FG};
use crate::font::{fit_px, Align, Fonts, SCRIPT};
use crate::gfx::{segment_distance, triangle_distance, Painter, Style};
use crate::menu::{AppIcon, Draft, PlayMode, Setup, Value};
use crate::nest::{ChargeView, ClockView, Label, NestFace, Screen};
use crate::pigs::{Locked, Outcome, Symbol, Throw, Token};
use crate::smoke::Special;

/// The lit area's half-size in canvas units.
pub const ACTIVE: f32 = 83.0;

/// Everything a screen needs to draw one face.
pub struct Ctx<'a, 'p, A: AssetStore, T: Target = Grey96> {
    pub painter: &'a mut Painter<'p, T>,
    pub fonts: &'a mut Fonts,
    pub assets: &'a mut A,
}

impl<A: AssetStore, T: Target> Ctx<'_, '_, A, T> {
    fn text(&mut self, text: &str, x: f32, y: f32, px: u16, style: Style) {
        self.fonts
            .draw(self.assets, self.painter, text, x, y, px, Align::Center, style);
    }

    #[allow(clippy::too_many_arguments)]
    fn script(&mut self, text: &str, x: f32, y: f32, px: u16, reveal: f32, style: Style) {
        self.fonts
            .draw_script(self.assets, self.painter, text, x, y, px, reveal, style);
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
pub(crate) const FACE_START: [u8; 6] = [3, 4, 1, 6, 2, 5];
pub(crate) const BOOT_STEP: f32 = 0.34;
pub(crate) const BOOT_STEPS: u32 = 5;
pub(crate) const BOOT_FADE: f32 = 0.4;
pub const LOOP_END: f32 = 0.25 + BOOT_STEP * BOOT_STEPS as f32;
/// Whole boot sequence, seconds.
pub const BOOT_DURATION: f32 = LOOP_END + 4.25;
/// The sugar cube (the lone centre pip, squared up) dissolves this long
/// after the loop.
pub const DISSOLVE_AT: f32 = 0.9;
/// The name, in the retro script the brand is set in.
pub const WORDMARK: &str = "Sugarcube";
/// The wordmark's canvas size (one of the pack's script sizes).
const WORDMARK_PX: u16 = 34;
const PIP_GRID: f32 = ACTIVE * 0.46;
const PIP_RADIUS: f32 = ACTIVE * 0.15;
const PIP_GLOW: f32 = 10.0;

/// The boot's colours: all one grey on the 96×96 die; the 64×64 colour
/// panel plays the same animation in its palette.
#[derive(Clone, Copy)]
pub struct BootInk {
    /// The pips and the sugar cube.
    pub pips: Color,
    /// The crystals the cube dissolves into.
    pub crystals: Color,
    /// The wordmark.
    pub word: Color,
    /// The glint that rides the pen and twinkles on the last letter.
    pub glint: Color,
}

const GREY_INK: BootInk = BootInk {
    pips: Color::grey(FG),
    crystals: Color::grey(FG),
    word: Color::grey(FG),
    glint: Color::grey(FG),
};

#[allow(clippy::too_many_arguments)]
fn draw_pips<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    from: u8,
    to: u8,
    t: f32,
    alpha: f32,
    scale: f32,
    ink: Color,
) {
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
        let style = Style::color(ink, alpha * (sc * 1.5).min(1.0), PIP_GLOW);
        c.painter.fill_circle(x, y, PIP_RADIUS * sc, style);
    }
}

/// One face of the boot animation. `t` is seconds since boot, `index` the
/// face index, `top` whether this face was on top when boot started.
pub fn draw_boot<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, index: usize, top: bool, t: f32) {
    draw_boot_in(c, index, top, t, GREY_INK);
}

/// [`draw_boot`] in the colours of `ink`.
pub fn draw_boot_in<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    index: usize,
    top: bool,
    t: f32,
    ink: BootInk,
) {
    // A slight stagger around the cube.
    let t = t - if top { 0.0 } else { index as f32 * 0.05 };
    if t < 0.0 {
        return;
    }
    if top && t >= LOOP_END {
        draw_boot_finale(c, t - LOOP_END, ink);
        return;
    }
    // The top face starts on 6 so the loop lands it on 5.
    let v0 = if top { 6 } else { FACE_START[index] };
    let val = |k: u32| ((v0 as u32 - 1 + k) % 6 + 1) as u8;
    let appear = (t / 0.25).min(1.0);
    let steps_t = t - 0.25;
    let fade = ((steps_t - BOOT_STEP * BOOT_STEPS as f32) / BOOT_FADE).clamp(0.0, 1.0);
    if steps_t < 0.0 {
        draw_pips(c, v0, v0, 0.0, appear, appear, ink.pips);
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
    draw_pips(c, from, to, local, 1.0 - fade, 1.0 - 0.6 * fade, ink.pips);
}

/// Top face after the loop: the corner pips shoot outward, the centre pip
/// squares up into a sugar cube, wiggles and dissolves into glittering
/// crystals, and the name writes itself on in script with a sparkle.
fn draw_boot_finale<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, u: f32, ink: BootInk) {
    if u < 0.35 {
        let e = powf(u / 0.35, 2.0);
        let d = PIP_GRID * (1.0 + 1.4 * e);
        let style = Style::color(ink.pips, 1.0 - e, PIP_GLOW);
        for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            c.painter
                .fill_circle(x * d, y * d, PIP_RADIUS * (1.0 - 0.4 * e), style);
        }
    }
    if u < DISSOLVE_AT + 0.2 {
        draw_sugar_cube(c, u, ink.pips);
    }
    if u > DISSOLVE_AT {
        draw_crystals(c, u - DISSOLVE_AT, ink.crystals);
    }
    let out = if u < 3.8 {
        1.0
    } else {
        (1.0 - (u - 3.8) / 0.4).max(0.0)
    };
    let reveal = smoothstep(((u - 1.15) / 0.95).clamp(0.0, 1.0));
    if reveal > 0.0 && out > 0.0 {
        c.script(
            WORDMARK,
            0.0,
            0.0,
            WORDMARK_PX,
            reveal,
            Style::color(ink.word, out, 14.0),
        );
        let width = c.fonts.measure_in(c.assets, SCRIPT, WORDMARK, WORDMARK_PX);
        // A glint rides the pen, then twinkles once on the last letter.
        let pen = -width / 2.0 + width * reveal;
        if reveal < 1.0 {
            draw_sparkle(
                c,
                pen,
                -6.0,
                5.0 * sinf(reveal * core::f32::consts::PI),
                out,
                ink.glint,
            );
        }
        let tw = (u - 2.25) / 0.5;
        if (0.0..1.0).contains(&tw) {
            draw_sparkle(
                c,
                width / 2.0 - 2.0,
                -24.0,
                9.0 * sinf(tw * core::f32::consts::PI),
                out,
                ink.glint,
            );
        }
    }
}

/// The centre pip becomes a sugar cube: a circle whose corners square up
/// with a little bounce, a wiggle, then it shrinks away as it dissolves.
fn draw_sugar_cube<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, u: f32, ink: Color) {
    let square = smoothstep((u / 0.4).min(1.0));
    let bounce = 1.0 + 0.18 * sinf((u / 0.4).min(1.0) * core::f32::consts::PI);
    let dissolve = ((u - DISSOLVE_AT) / 0.2).clamp(0.0, 1.0);
    let half = PIP_RADIUS * bounce * (1.0 - 0.15 * square) * (1.0 - dissolve);
    if half <= 0.2 {
        return;
    }
    // Rounded to a circle at first, then a cube with soft corners.
    let round = half * (1.0 - 0.72 * square);
    let wiggle = if u > 0.4 {
        0.22 * sinf((u - 0.4) * 22.0) * (1.0 - ((u - 0.4) / (DISSOLVE_AT - 0.4)).min(1.0))
    } else {
        0.0
    };
    let (sn, cs) = (sinf(wiggle), cosf(wiggle));
    let e = half * 1.5;
    let style = Style::color(ink, 1.0 - dissolve, PIP_GLOW);
    c.painter.shape((-e, -e, e, e), style, |x, y| {
        let (rx, ry) = (x * cs + y * sn, -x * sn + y * cs);
        rounded_box(rx, ry, half, round)
    });
}

/// Signed distance to a square of half-size `half` with corner radius `r`.
fn rounded_box(x: f32, y: f32, half: f32, r: f32) -> f32 {
    let (qx, qy) = (libm::fabsf(x) - half + r, libm::fabsf(y) - half + r);
    let outside = sqrtf(qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0));
    outside + qx.max(qy).min(0.0) - r
}

/// Sugar crystals thrown out as the cube dissolves: little tumbling squares
/// that drift outward, slow down, twinkle and fade. `v` is seconds since
/// the dissolve.
fn draw_crystals<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, v: f32, ink: Color) {
    const N: usize = 14;
    const LIFE: f32 = 1.5;
    if v >= LIFE {
        return;
    }
    let travel = 1.0 - powf(1.0 - v / LIFE, 3.0);
    let mut crystals = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); N];
    for (i, k) in crystals.iter_mut().enumerate() {
        // Fixed, uneven angles and reaches so the spray looks scattered.
        let a = i as f32 * 2.39996 + 0.4;
        let reach = ACTIVE * (0.35 + 0.45 * frac(i as f32 * 0.618_034));
        let r = PIP_RADIUS * 0.4 + reach * travel;
        let size = 1.6 + 1.6 * frac(i as f32 * 0.414_214);
        *k = (
            cosf(a) * r,
            sinf(a) * r,
            size * (1.0 - 0.5 * v / LIFE),
            a + v * 6.0,
        );
    }
    let twinkle = 0.75 + 0.25 * sinf(v * 30.0);
    let alpha = (1.0 - v / LIFE) * twinkle;
    let e = ACTIVE;
    c.painter
        .shape((-e, -e, e, e), Style::color(ink, alpha, 4.0), |x, y| {
            let mut d = f32::MAX;
            for &(cx, cy, h, spin) in &crystals {
                let (dx, dy) = (x - cx, y - cy);
                if libm::fabsf(dx) > h * 2.0 || libm::fabsf(dy) > h * 2.0 {
                    d = d.min(libm::fabsf(dx).max(libm::fabsf(dy)) - h * 1.5);
                    continue;
                }
                let (sn, cs) = (sinf(spin), cosf(spin));
                d = d.min(rounded_box(dx * cs + dy * sn, -dx * sn + dy * cs, h, h * 0.25));
            }
            d
        });
}

fn frac(x: f32) -> f32 {
    x - floorf(x)
}

/// A four-point sparkle of arm length `r` centred on `(x, y)`.
fn draw_sparkle<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, x: f32, y: f32, r: f32, alpha: f32, ink: Color) {
    if r <= 0.3 || alpha <= 0.0 {
        return;
    }
    let long: [&[(f32, f32)]; 2] = [&[(x - r, y), (x + r, y)], &[(x, y - r), (x, y + r)]];
    c.painter
        .stroke_paths(&long, (r * 0.28).max(1.2), Style::color(ink, alpha, 8.0));
    c.painter
        .fill_circle(x, y, r * 0.3, Style::color(ink, alpha, 0.0));
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

// ---------- table screens (brief 3, 1.2) ----------
//
// Read from across the table: T1, the answer in 1–3 characters or one
// glyph, over at most one T2 word, and nothing smaller. Sizes are canvas px
// whose capitals (0.70 of the size, × K panel px each) land on the 34 mm
// die's tiers.

/// T1 for 1–2 characters: 28.3 px caps.
pub const T1_PX: u16 = 70;
/// T1 for 3 characters, its floor: 20.2 px caps.
pub const T1_FLOOR_PX: u16 = 50;
/// T2: 14.2 px caps.
pub const T2_PX: u16 = 35;
/// H1, a held screen's value: 17.0 px caps.
pub const H1_PX: u16 = 42;
/// H2, a held screen's titles and labels: 7.3 px caps.
pub const H2_PX: u16 = 18;
/// T1's and T2's cap heights in canvas units, and the gap between them.
const T1_CAP: f32 = 49.0;
const T2_CAP: f32 = 24.5;
const T1_GAP: f32 = 12.0;

/// The centres of T1 (`h1` tall) and T2, centred together on the face.
fn table_layout(h1: f32) -> (f32, f32) {
    let top = -(h1 + T1_GAP + T2_CAP) / 2.0;
    (top + h1 / 2.0, top + h1 + T1_GAP + T2_CAP / 2.0)
}

/// T1's size for `text`: the target for 1–2 characters, the floor for 3.
fn t1_px(text: &str) -> u16 {
    if text.chars().count() <= 2 {
        T1_PX
    } else {
        T1_FLOOR_PX
    }
}

/// A T1 answer centred on `y`.
fn draw_t1<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, text: &str, y: f32, value: u8, alpha: f32) {
    c.text(text, 0.0, y, t1_px(text), Style::new(value, alpha, 18.0));
}

/// A T2 word centred on `y`.
fn draw_t2<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, text: &str, y: f32, value: u8, alpha: f32) {
    c.text(text, 0.0, y, T2_PX, Style::new(value, alpha, 8.0));
}

/// A player's token as T1, centred on `y`: an initial, or a symbol drawn
/// as tall as T1's capitals.
fn draw_t1_token<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, token: Token, y: f32, alpha: f32) {
    match (token.initial(), token.as_symbol()) {
        (_, Some(symbol)) => {
            draw_symbol(c, symbol, 0.0, y, T1_CAP, Style::new(FG, alpha, 12.0));
            c.painter.note_icon(T1_CAP, alpha);
        }
        (Some(ch), _) => {
            let mut s: String<2> = String::new();
            let _ = s.push(ch);
            draw_t1(c, &s, y, FG, alpha);
        }
        _ => {}
    }
}

/// The wake/setup label (a table screen): the setup's icon (a die's solid,
/// a bomb) as T1, and the dice, or `ready`, as T2. Pass the Pot shows the
/// bills in hand.
pub fn draw_wake_label<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, setup: Setup, label: &str, alpha: f32) {
    let _ = label;
    if let Setup::Roll(DieKind::PassThePot, n) = setup {
        draw_bills_in_hand(c, n, alpha);
        return;
    }
    let r = 27.0;
    let (y1, y2) = table_layout(2.0 * r);
    crate::icons::draw_setup_icon(c, setup, 0.0, y1, r, alpha);
    c.painter.note_icon(2.0 * r, alpha);
    let word = match setup {
        Setup::Roll(die, n) => crate::table::dice(die, n),
        _ => {
            let mut s = String::new();
            let _ = s.push_str("ready");
            s
        }
    };
    draw_t2(c, &word, y2, FG, alpha);
}

/// Pass the Pot between rolls: the bills in hand as T1, `bills` as T2.
fn draw_bills_in_hand<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, n: u8, alpha: f32) {
    let mut digits: String<2> = String::new();
    let _ = write!(digits, "{n}");
    let (y1, y2) = table_layout(T1_CAP);
    draw_t1(c, &digits, y1, FG, alpha);
    draw_t2(c, if n == 1 { "bill" } else { "bills" }, y2, FG, alpha);
}

/// Pig Toss between throws: whose go it is as T1, `turn` as T2 (or once
/// someone has won, `wins`).
pub fn draw_pigs_label<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    setup: Setup,
    token: Token,
    won: bool,
    alpha: f32,
) {
    let _ = setup;
    let (y1, y2) = table_layout(T1_CAP);
    draw_t1_token(c, token, y1, alpha);
    draw_t2(c, if won { "wins" } else { "turn" }, y2, FG, alpha);
}

// ---------- players' tokens ----------

/// A player's symbol, centred on `(cx, cy)`, `h` canvas units tall.
pub fn draw_symbol<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    symbol: Symbol,
    cx: f32,
    cy: f32,
    h: f32,
    style: Style,
) {
    use core::f32::consts::PI;
    let u = h / 2.0;
    let p = |x: f32, y: f32| (cx + x * u, cy + y * u);
    let dark = Style::new(0, style.alpha, 0.0);
    let painter = &mut *c.painter;
    let rect = |painter: &mut Painter<T>, x: f32, y: f32, w: f32, hh: f32, st: Style| {
        painter.fill_rect(cx + x * u, cy + y * u, w * u, hh * u, st)
    };
    // A four-sided shape, as two triangles.
    let quad = |painter: &mut Painter<T>, q: [(f32, f32); 4], st: Style| {
        painter.fill_triangle([p(q[0].0, q[0].1), p(q[1].0, q[1].1), p(q[2].0, q[2].1)], st);
        painter.fill_triangle([p(q[0].0, q[0].1), p(q[2].0, q[2].1), p(q[3].0, q[3].1)], st);
    };
    match symbol {
        Symbol::Hat => {
            rect(painter, -0.62, -0.95, 1.24, 1.55, style);
            rect(painter, -1.0, 0.55, 2.0, 0.34, style);
            rect(painter, -0.62, 0.2, 1.24, 0.2, dark);
        }
        Symbol::Car => {
            quad(
                painter,
                [(-0.55, -0.1), (-0.3, -0.62), (0.35, -0.62), (0.62, -0.1)],
                style,
            );
            rect(painter, -1.0, -0.12, 2.0, 0.62, style);
            for x in [-0.55, 0.55] {
                let (wx, wy) = p(x, 0.52);
                painter.fill_circle(wx, wy, 0.34 * u, dark);
                painter.fill_circle(wx, wy, 0.26 * u, style);
                painter.fill_circle(wx, wy, 0.1 * u, dark);
            }
        }
        Symbol::Boot => {
            rect(painter, -0.62, -1.0, 0.8, 1.4, style);
            rect(painter, -0.62, 0.2, 1.3, 0.6, style);
            let (tx, ty) = p(0.68, 0.5);
            painter.fill_circle(tx, ty, 0.3 * u, style);
            rect(painter, -0.62, 0.8, 1.62, 0.18, style);
            rect(painter, -0.75, -1.0, 1.06, 0.2, style);
        }
        Symbol::Boat => {
            quad(
                painter,
                [(-1.0, 0.3), (1.0, 0.3), (0.62, 0.9), (-0.62, 0.9)],
                style,
            );
            rect(painter, -0.05, -1.0, 0.1, 1.3, style);
            painter.fill_triangle([p(0.12, -0.92), p(0.12, 0.18), p(0.85, 0.18)], style);
            painter.fill_triangle([p(-0.12, -0.62), p(-0.12, 0.18), p(-0.7, 0.18)], style);
        }
        Symbol::Crown => {
            rect(painter, -0.9, -0.05, 1.8, 0.75, style);
            painter.fill_triangle([p(-0.9, 0.0), p(-0.9, -0.7), p(-0.3, 0.0)], style);
            painter.fill_triangle([p(-0.42, 0.0), p(0.0, -0.95), p(0.42, 0.0)], style);
            painter.fill_triangle([p(0.3, 0.0), p(0.9, -0.7), p(0.9, 0.0)], style);
            for (x, y) in [(-0.9, -0.7), (0.0, -0.95), (0.9, -0.7)] {
                let (bx, by) = p(x, y);
                painter.fill_circle(bx, by, 0.14 * u, style);
            }
            rect(painter, -0.9, 0.3, 1.8, 0.14, dark);
        }
        Symbol::Star => {
            let point = |k: i32, r: f32| {
                let a = -PI / 2.0 + k as f32 * PI / 5.0;
                p(r * cosf(a), 0.1 + r * sinf(a))
            };
            for k in 0..5 {
                let (tip, l, r) = (point(2 * k, 1.05), point(2 * k - 1, 0.44), point(2 * k + 1, 0.44));
                painter.fill_triangle([tip, l, r], style);
                painter.fill_triangle([p(0.0, 0.1), l, r], style);
            }
        }
    }
}

// ---------- results (C6) ----------

/// A roll's result: the total as T1 over the dice, or `MAX` / `DUD`, as
/// T2 (C6). A dud is the dud grey. Pass the Pot shows its dice as glyphs.
pub fn draw_result<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    record: &RollRecord,
    special: Option<Special>,
    alpha: f32,
) {
    let value = if special == Some(Special::Dud) { DUD } else { FG };
    if !record.die.is_numeric() {
        draw_pot_result(c, &record.values, value, alpha);
        return;
    }
    let total = crate::table::total(record.total());
    let dice = crate::table::dice(record.die, record.values.len() as u8);
    let word = match special {
        Some(Special::Max) => "MAX",
        Some(Special::Dud) => "DUD",
        None => dice.as_str(),
    };
    let (y1, y2) = table_layout(T1_CAP);
    draw_t1(c, &total, y1, value, alpha);
    draw_t2(c, word, y2, value, alpha * 0.85);
}

/// A Pass the Pot result: the dice as T1 glyphs in a row (arrows pass left
/// or right, the pot glyph feeds the pot, a dot keeps), and as T2 `keep`
/// or how many bills leave the hand, `−2`.
fn draw_pot_result<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, values: &[u8], value: u8, alpha: f32) {
    let n = values.len().max(1);
    let sz = match n {
        1 => 70.0,
        2 => 58.0,
        _ => 46.0,
    };
    let h = sz * 0.84;
    let (y1, y2) = table_layout(h);
    let gap = sz * 1.15;
    let style = Style::new(value, alpha, 14.0);
    for (i, &v) in values.iter().enumerate() {
        let x = (i as f32 - (n as f32 - 1.0) / 2.0) * gap;
        match PotFace::from_raw(v) {
            PotFace::Keep => c.painter.fill_circle(x, y1, sz * 0.2, style),
            PotFace::Pot => draw_pot(c, x, y1, sz, style),
            dir @ (PotFace::Left | PotFace::Right) => {
                let d = if dir == PotFace::Right { 1.0 } else { -1.0 };
                draw_pass_arrow(c, x, y1, sz, d, style);
            }
        }
    }
    c.painter.note_icon(h, alpha);
    draw_t2(c, &crate::table::pot(values), y2, value, alpha * 0.85);
}

/// A bold arrow for passing a bill: a round-capped shaft into a solid,
/// slightly rounded head, drawn as one shape so it glows and fades as one.
fn draw_pass_arrow<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    x: f32,
    y: f32,
    sz: f32,
    d: f32,
    style: Style,
) {
    let (l, hl, hw) = (sz * 0.44, sz * 0.36, sz * 0.42);
    let (shaft_w, round) = (sz * 0.2, sz * 0.035);
    let tip = x + d * (l - round);
    let base = x + d * (l - hl);
    let head = [(tip, y), (base, y - hw), (base, y + hw)];
    let (a, b) = ((x - d * l + d * shaft_w / 2.0, y), (base + d * sz * 0.05, y));
    let e = l + shaft_w;
    c.painter
        .shape((x - e, y - hw - round, x + e, y + hw + round), style, |px, py| {
            let shaft = segment_distance(px, py, a, b) - shaft_w / 2.0;
            shaft.min(triangle_distance(px, py, head) - round)
        });
}

/// The pot: a round-bellied cauldron with handles and feet, and a coin
/// dropping in.
fn draw_pot<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, x: f32, y: f32, sz: f32, style: Style) {
    const N: usize = 14;
    const EXP: f32 = 0.8;
    let (bw, bh) = (sz * 0.38, sz * 0.42);
    let rim = y - sz * 0.02;
    let line = sz * 0.08;
    // The belly: a round-bottomed half-superellipse hanging from the rim.
    let mut belly = [(0.0, 0.0); N + 1];
    for (k, p) in belly.iter_mut().enumerate() {
        let t = core::f32::consts::PI * k as f32 / N as f32;
        let (cx, sy) = (cosf(t), sinf(t));
        let fx = if cx < 0.0 { -powf(-cx, EXP) } else { powf(cx, EXP) };
        *p = (x + bw * fx, rim + bh * powf(sy, EXP));
    }
    // Dim inside, like the bomb's.
    let n = 2.0 / EXP;
    let inner = Style::color(style.color, style.alpha * 0.22, 0.0);
    c.painter.shape((x - bw, rim, x + bw, rim + bh), inner, |px, py| {
        let (u, v) = (fabsf(px - x) / bw, ((py - rim) / bh).max(0.0));
        let f = powf(powf(u, n) + powf(v, n), 1.0 / n) - 1.0;
        (f * bw).max(rim - py)
    });
    let lip = bw + sz * 0.08;
    let rim_line = [(x - lip, rim), (x + lip, rim)];
    let ear = |s: f32| {
        [
            (x + s * bw * 0.98, rim + sz * 0.1),
            (x + s * (bw + sz * 0.12), rim + sz * 0.13),
            (x + s * (bw + sz * 0.12), rim + sz * 0.22),
            (x + s * bw * 0.93, rim + sz * 0.25),
        ]
    };
    let (ear_l, ear_r) = (ear(-1.0), ear(1.0));
    // Feet splay out from the belly's underside.
    let under = rim + bh * powf(1.0 - powf(0.5, n), 1.0 / n);
    let foot = |s: f32| {
        [
            (x + s * bw * 0.5, under - line * 0.3),
            (x + s * bw * 0.66, under + sz * 0.1),
        ]
    };
    let (foot_l, foot_r) = (foot(-1.0), foot(1.0));
    c.painter.stroke_paths(
        &[&belly, &rim_line, &ear_l, &ear_r, &foot_l, &foot_r],
        line,
        style,
    );
    // The coin, on its way in.
    c.painter.fill_circle(x, rim - sz * 0.3, sz * 0.13, style);
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

/// A heart, centred on `(cx, cy)`, `h` canvas units tall: a smooch.
fn draw_heart<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, cx: f32, cy: f32, h: f32, style: Style) {
    let r = h * 0.28;
    let top = cy - h / 2.0 + r;
    c.painter.fill_circle(cx - r * 0.95, top, r, style);
    c.painter.fill_circle(cx + r * 0.95, top, r, style);
    c.painter.fill_triangle(
        [
            (cx - r * 1.9, top + r * 0.3),
            (cx + r * 1.9, top + r * 0.3),
            (cx, cy + h / 2.0),
        ],
        style,
    );
}

/// What a throw says once the pigs have settled, `t` seconds after the die
/// landed. First the throw: its points as T1 (`+15`) over what scored
/// (`Strut`, `Twin`) as T2; an oops is the dud grey, the turn's points lost
/// (`−20`, or `0`) over `OOPS`; a smooch a heart over the banked score lost
/// (`−40`, or `kiss`). Then the resting screen: the player's token over the
/// turn so far (`+35`), or after an oops or a smooch, whose go it is now.
/// The pigs themselves are [`crate::pigfx`].
pub fn draw_pig_score<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    throw: &Throw,
    t: f32,
    next_player: u8,
    tokens: &[Token],
    alpha: f32,
) {
    use pig_score::*;
    let token = |p: u8| tokens.get(p as usize).copied().unwrap_or(Token::default_for(p));
    let rest = ramp(t, PROMPT_AT, 0.3);
    let first = 1.0 - rest;
    let pop = ramp(t, POP_AT, 0.35);
    let word_a = alpha * ramp(t, LABEL_AT, 0.25) * first;
    let (y1, y2) = table_layout(T1_CAP);
    if pop > 0.0 && first > 0.0 {
        let a = alpha * (pop * 3.0).min(1.0) * first;
        let over = 1.0 - pop;
        let grow = 1.0 + 0.4 * over * over;
        match throw.outcome {
            Outcome::Score(n) => {
                let big = crate::table::signed(n as i32);
                c.shifted(0.0, y1, grow, |c| draw_t1(c, &big, 0.0, FG, a));
                draw_t2(c, crate::table::throw_word(throw), y2, FG, word_a);
            }
            Outcome::Bust => {
                let lost = crate::table::signed(-(throw.turn_before as i32));
                c.shifted(0.0, y1, grow, |c| draw_t1(c, &lost, 0.0, DUD, a));
                draw_t2(c, "OOPS", y2, DUD, word_a);
            }
            Outcome::Smooch => {
                draw_heart(c, 0.0, y1, T1_CAP * grow, Style::new(FG, a, 12.0));
                c.painter.note_icon(T1_CAP, a);
                let word = if throw.banked_before > 0 {
                    crate::table::signed(-(throw.banked_before as i32))
                } else {
                    let mut s = String::new();
                    let _ = s.push_str("kiss");
                    s
                };
                draw_t2(c, &word, y2, FG, word_a);
            }
        }
    }
    if rest <= 0.0 {
        return;
    }
    let a = alpha * rest;
    match throw.outcome {
        Outcome::Score(n) if !throw.won => {
            draw_t1_token(c, token(throw.player), y1, a);
            draw_t2(
                c,
                &crate::table::signed((throw.turn_before + n) as i32),
                y2,
                FG,
                a,
            );
        }
        Outcome::Score(_) => {}
        _ => {
            draw_t1_token(c, token(next_player), y1, a);
            draw_t2(c, "turn", y2, FG, a);
        }
    }
}

/// When each part of the lock-in starts, in seconds after the tap.
pub mod lock_in {
    /// The padlock closes.
    pub const SNAP_AT: f32 = 0.55;
    /// The banked points turn into the new total, and it counts up.
    pub const COUNT_AT: f32 = 0.8;
    pub const COUNT_S: f32 = 0.9;
    /// Who has the die next.
    pub const NEXT_AT: f32 = 2.0;
}

/// A bank locking in, `t` seconds after the hold: the banked points as T1
/// over a padlock (T2) that swings shut with a flash and a burst of sparks;
/// the points turn into the player's new total and count up; then whose go
/// it is, as between turns. `fade` (0–1) takes it away when it has been up
/// a while.
pub fn draw_locked<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, l: &Locked, next: Token, t: f32, fade: f32) {
    use core::f32::consts::PI;
    use lock_in::*;
    let alpha = ramp(t, 0.0, 0.15) * fade;
    let next_a = ramp(t, NEXT_AT, 0.3);
    let (y1, y2) = table_layout(T1_CAP);
    if next_a >= 1.0 {
        draw_t1_token(c, next, y1, alpha);
        draw_t2(c, "turn", y2, FG, alpha);
        return;
    }
    let alpha = alpha * (1.0 - next_a);
    let style = |a: f32, glow: f32| Style::new(FG, alpha * a, glow);

    // The number: the points, then the new total counting up.
    let count = ramp(t, COUNT_AT, COUNT_S);
    let eased = 1.0 - (1.0 - count) * (1.0 - count);
    let mut big: String<8> = String::new();
    let _ = if t < COUNT_AT {
        write!(big, "+{}", l.points)
    } else {
        write!(big, "{}", l.before + roundf(l.points as f32 * eased) as u16)
    };
    draw_t1(c, &big, y1, FG, alpha);

    // The padlock as T2, 26 units tall: open at first, then it drops shut.
    let k = 0.55;
    let closing = (t / SNAP_AT).clamp(0.0, 1.0);
    let lift = 8.0 * (1.0 - closing * closing);
    let body_top = y2 - 13.0 + 26.0 * 0.38;
    let snapped = t >= SNAP_AT;
    let flash = if snapped {
        (1.0 - (t - SNAP_AT) / 0.5).max(0.0)
    } else {
        0.0
    };
    let lock = style(1.0, 6.0 + 14.0 * flash);
    let dark = Style::new(0, alpha, 0.0);
    let (sr, sw) = (13.0 * k, 4.5 * k);
    let shackle = body_top - lift;
    c.painter.stroke_arc(0.0, shackle, sr, PI, 2.0 * PI, sw, lock);
    c.painter.stroke_paths(
        &[
            &[(-sr, shackle), (-sr, shackle + 11.0 * k)],
            &[(sr, shackle), (sr, shackle + 11.0 * k)],
        ],
        sw,
        lock,
    );
    c.painter.fill_rect(-21.0 * k, body_top, 42.0 * k, 31.0 * k, lock);
    c.painter.fill_circle(0.0, body_top + 12.0 * k, 4.5 * k, dark);
    c.painter.note_icon(26.0, alpha);
    if snapped {
        let u = ((t - SNAP_AT) / 0.55).min(1.0);
        if u < 1.0 {
            for i in 0..10 {
                let a = i as f32 * PI / 5.0 + 0.3;
                let r = 20.0 + 30.0 * u * (0.7 + 0.3 * ((i % 3) as f32 / 2.0));
                let dot = Style::new(FG, alpha * (1.0 - u), 4.0);
                c.painter
                    .fill_circle(cosf(a) * r, y2 + sinf(a) * r, 2.6 * (1.0 - 0.6 * u), dot);
            }
        }
    }
}

/// The win: once a winning throw's score has counted up, it fades and a
/// happy pig takes over.
pub mod pig_win {
    use crate::pigs::Throw;

    /// Seconds after the die lands that the win screen starts: the throw's
    /// points have popped in and the total has counted up.
    pub const AT: f32 = 3.2;
    /// How long the throw's score takes to fade out before it.
    pub const FADE_S: f32 = 0.4;

    /// How much of the throw's score to show, `t` seconds after landing:
    /// all of it, unless the throw won, when it fades out before the win.
    pub fn score_fade(throw: Option<&Throw>, t: f32) -> f32 {
        if throw.is_some_and(|t| t.won) {
            1.0 - super::ramp(t, AT - FADE_S, FADE_S)
        } else {
            1.0
        }
    }
}

/// The win screen, `t` seconds in: the winner's token pops in as T1 with a
/// burst of sparks, over `wins`. `fade` (0–1) takes it away when it has
/// been up a while.
pub fn draw_pig_win<A: AssetStore, T: Target>(
    c: &mut Ctx<A, T>,
    winner: Token,
    total: u16,
    t: f32,
    fade: f32,
) {
    use core::f32::consts::PI;
    let _ = total;
    let alpha = ramp(t, 0.0, 0.2) * fade;
    let (y1, y2) = table_layout(T1_CAP);
    let pop = ramp(t, 0.0, 0.45);
    let over = 1.0 - pop;
    let scale = (1.0 - over * over * over) * (1.0 + 0.25 * sinf(pop * PI));
    c.shifted(0.0, y1, scale.max(0.01), |c| draw_t1_token(c, winner, 0.0, alpha));
    let u = ((t - 0.35) / 0.7).clamp(0.0, 1.0);
    if t > 0.35 && u < 1.0 {
        for k in 0..12 {
            let a = k as f32 * PI / 6.0 + 0.2;
            let r = 36.0 + 42.0 * u * (0.7 + 0.3 * ((k % 3) as f32 / 2.0));
            let dot = Style::new(FG, alpha * (1.0 - u), 4.0);
            c.painter
                .fill_circle(cosf(a) * r, y1 + sinf(a) * r, 2.6 * (1.0 - 0.6 * u), dot);
        }
    }
    draw_t2(c, "wins", y2, FG, alpha * ramp(t, 0.6, 0.3));
}

// ---------- Hot Potato ----------

/// A lit fuse: a glow (T1) that swells with `heat` (0–1) and flashes on
/// each tick (`pulse`, 0–1), over `PASS` (T2).
pub fn draw_fuse<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, heat: f32, pulse: f32) {
    let core = 20.0 + 14.0 * heat;
    let (y1, y2) = table_layout(2.0 * 27.0);
    c.painter.fill_circle(
        0.0,
        y1,
        core,
        Style::new(FG, 0.30 + 0.45 * heat, 10.0 + 14.0 * heat),
    );
    c.painter
        .fill_circle(0.0, y1, core * 0.55, Style::new(FG, 0.55 + 0.45 * pulse, 12.0));
    if pulse > 0.0 {
        c.painter.fill_circle(
            0.0,
            y1,
            core + 10.0 * (1.0 - pulse),
            Style::new(FG, 0.35 * pulse, 0.0),
        );
    }
    c.painter.note_icon(2.0 * core, 1.0);
    draw_t2(c, "PASS", y2, FG, 1.0);
}

/// The fuse ran out, `t` seconds ago: a shockwave, then a burst (T1) over
/// `BOOM` (T2).
pub fn draw_boom<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, t: f32) {
    use core::f32::consts::PI;
    let p = (t / 0.7).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - p) * (1.0 - p) * (1.0 - p);
    if p < 1.0 {
        c.painter
            .fill_circle(0.0, 0.0, 8.0 + 92.0 * e, Style::new(FG, 0.9 * (1.0 - p), 20.0));
    }
    let fade = (1.0 - (t - 4.6) / 1.0).clamp(0.0, 1.0);
    let a = (t / 0.12).min(1.0) * fade;
    let r = 28.0;
    let (y1, y2) = table_layout(2.0 * r);
    // An eight-pointed burst.
    let style = Style::new(FG, a, 16.0);
    for k in 0..8 {
        let ang = k as f32 * PI / 4.0;
        let (l, rr) = (ang - PI / 8.0, ang + PI / 8.0);
        c.painter.fill_triangle(
            [
                (cosf(ang) * r, y1 + sinf(ang) * r),
                (cosf(l) * r * 0.45, y1 + sinf(l) * r * 0.45),
                (cosf(rr) * r * 0.45, y1 + sinf(rr) * r * 0.45),
            ],
            style,
        );
    }
    c.painter.fill_circle(0.0, y1, r * 0.5, style);
    c.painter.note_icon(2.0 * r, a);
    draw_t2(c, "BOOM", y2, FG, a);
}

// ---------- menu (C3, C4) ----------

/// How far menu content slides during a tip, canvas units.
pub const TIP_SLIDE: f32 = 120.0;

/// One menu page (a held screen, brief 3, 1.3): the setup and the title at
/// the top, the value between ▲ and ▼ (or a caption under it), and the page
/// dots. Battery has a page of its own in Settings.
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
    c.shifted(ox, oy, scale, |c| {
        c.text(&view.status, 0.0, -66.0, H2_PX, Style::new(FG, 0.6 * alpha, 0.0));
        c.text(&view.title, 0.0, -44.0, H2_PX, Style::new(FG, 0.8 * alpha, 0.0));
        let arrows = Style::new(FG, 0.55 * alpha, 0.0);
        let (w, h) = (4.5, 4.0);
        if view.arrows {
            c.painter
                .fill_triangle([(0.0, -26.0 - h), (w, -26.0 + h), (-w, -26.0 + h)], arrows);
            if view.caption.is_none() {
                c.painter
                    .fill_triangle([(-w, 46.0 - h), (w, 46.0 - h), (0.0, 46.0 + h)], arrows);
            }
        }
        let value_y = if view.caption.is_some() { 6.0 } else { 10.0 };
        match &view.value {
            Value::Text(t) => {
                c.text(
                    t,
                    0.0,
                    value_y,
                    fit_px(t, 52, 150.0).max(H1_PX),
                    Style::new(FG, alpha, 14.0),
                );
            }
            Value::Token(t) => match (t.initial(), t.as_symbol()) {
                (_, Some(symbol)) => draw_symbol(c, symbol, 0.0, value_y, 46.0, Style::new(FG, alpha, 14.0)),
                (Some(ch), _) => {
                    let mut s: String<4> = String::new();
                    let _ = s.push(ch);
                    c.text(&s, 0.0, value_y, 52, Style::new(FG, alpha, 14.0));
                }
                _ => {}
            },
            Value::App(icon) => {
                let setup = match icon {
                    AppIcon::Game(PlayMode::Dice) => Some(Setup::Roll(m.die, 1)),
                    AppIcon::Game(PlayMode::PassThePot) => Some(Setup::Roll(DieKind::PassThePot, 1)),
                    AppIcon::Game(PlayMode::HotPotato) => Some(Setup::HotPotato),
                    AppIcon::Game(PlayMode::PigToss) => Some(Setup::Pigs(2)),
                    AppIcon::Settings => None,
                };
                match setup {
                    Some(setup) => crate::icons::draw_setup_icon(c, setup, 0.0, 0.0, 30.0, alpha),
                    None => crate::icons::draw_gear(c, 0.0, 0.0, 30.0, alpha),
                }
            }
            Value::Lines([a, b]) => {
                c.text(a, 0.0, -2.0, 22, Style::new(FG, 0.8 * alpha, 0.0));
                c.text(b, 0.0, 26.0, 22, Style::new(FG, alpha, 10.0));
            }
        }
        if let Some(caption) = view.caption {
            c.text(caption, 0.0, 46.0, 20, Style::new(FG, 0.8 * alpha, 0.0));
        }

        for i in 0..view.dots {
            let x = (i as f32 - (view.dots as f32 - 1.0) / 2.0) * 14.0;
            let a = if i == view.dot { 1.0 } else { 0.35 };
            c.painter
                .fill_circle(x, 66.0, 3.5, Style::new(FG, a * alpha, 0.0));
        }
    });
}

/// The hold ring: a rounded square just inside the lit area that fills
/// clockwise from 12 o'clock as a hold progresses (`p`, 0–1). `grow` pushes
/// it outward as it flashes away.
pub fn draw_hold_ring<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, p: f32, alpha: f32, grow: f32) {
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
pub fn draw_success<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, label: &str, nudge: &str, t: f32) {
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
    // The setup as H1 if it fits a line at that size, else H2.
    let px = fit_px(label, 50, 150.0);
    let px = if px >= H1_PX { px } else { 24 };
    c.text(label, 0.0, 6.0, px, Style::new(FG, a, 14.0));
    c.text(nudge, 0.0, 48.0, H2_PX + 4, Style::new(FG, a * 0.75, 0.0));
}

/// The whole-face flash when a setup is saved.
pub fn draw_flash<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
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
pub fn draw_nest<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, face: &NestFace) {
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
fn draw_charge<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, v: &ChargeView) {
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
fn draw_clock<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, v: &ClockView) {
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
fn draw_wisp<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, t: f32) {
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
fn draw_puff<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, p: f32) {
    for k in 0..7 {
        let a = core::f32::consts::TAU * k as f32 / 7.0 + 0.4;
        let d = 8.0 + 26.0 * p;
        let r = 9.0 + 10.0 * p;
        c.painter
            .fill_circle(cosf(a) * d, sinf(a) * d, r, Style::new(FG, 0.5 * (1.0 - p), 10.0));
    }
}

/// A chevron pointing along `dir`, centred on `at`.
fn draw_chevron<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, dir: (f32, f32), at: (f32, f32), alpha: f32) {
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
fn draw_arrow<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, dir: (f32, f32), at: (f32, f32), alpha: f32) {
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
fn draw_flip<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, t: f32) {
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
fn draw_no_power<A: AssetStore, T: Target>(c: &mut Ctx<A, T>) {
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
fn draw_fault<A: AssetStore, T: Target>(c: &mut Ctx<A, T>) {
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

/// What a hold will do, on the held face while the ring fills (brief 3,
/// 2.2.1): the action in a word (H2) over what it comes to (H1), or the
/// word alone as H1.
pub fn draw_hold_preview<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, word: &str, value: &str) {
    if value.is_empty() {
        c.text(word, 0.0, 10.0, H1_PX + 4, Style::new(FG, 1.0, 8.0));
    } else {
        c.text(word, 0.0, -30.0, 22, Style::new(FG, 0.8, 0.0));
        c.text(value, 0.0, 16.0, 52, Style::new(FG, 1.0, 8.0));
    }
}

/// A tap's hint along the bottom of the face, H2: `hold: bank`.
pub fn draw_tap_hint<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, text: &str, alpha: f32) {
    c.text(text, 0.0, 72.0, H2_PX, Style::new(FG, alpha * 0.8, 0.0));
}

/// The low-battery mark on the charging face: a lightning bolt as T1, `low`
/// as T2.
pub fn draw_bolt<A: AssetStore, T: Target>(c: &mut Ctx<A, T>, alpha: f32) {
    let h = 44.0;
    let (y1, y2) = table_layout(h);
    let s = h / 24.0;
    c.painter.stroke_polyline(
        &[
            (4.0 * s, y1 - 12.0 * s),
            (-5.0 * s, y1 + s),
            (5.0 * s, y1 - s),
            (-4.0 * s, y1 + 12.0 * s),
        ],
        4.0 * s * 0.8,
        Style::new(FG, alpha, 8.0),
    );
    c.painter.note_icon(h, alpha);
    draw_t2(c, "low", y2, FG, alpha);
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
