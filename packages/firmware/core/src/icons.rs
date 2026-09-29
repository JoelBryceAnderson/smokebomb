//! Line-art icons for the setup label: a wireframe of each die's solid, a
//! bomb for Hot Potato and a banknote for Pass the Pot. Coordinates are
//! canvas units around the icon's centre; `r` is the icon's radius.

use libm::{cosf, sinf};
use smokebomb_hal::AssetStore;
use smokebomb_shared::DieKind;

use crate::display::FG;
use crate::gfx::Style;
use crate::menu::Setup;
use crate::screens::Ctx;

const LINE: f32 = 2.6;
const GLOW: f32 = 8.0;
const TAU: f32 = core::f32::consts::TAU;

type P = (f32, f32);

fn at(cx: f32, cy: f32, r: f32, p: P) -> P {
    (cx + p.0 * r, cy + p.1 * r)
}

/// Corner `k` of an `n`-gon of radius 1, the first pointing straight up.
fn ngon(n: usize, k: usize, scale: f32, turn: f32) -> P {
    let a = -core::f32::consts::FRAC_PI_2 + turn + TAU * k as f32 / n as f32;
    (cosf(a) * scale, sinf(a) * scale)
}

/// Draw the icon for `setup`, centred on `(cx, cy)`.
pub fn draw_setup_icon<A: AssetStore>(c: &mut Ctx<A>, setup: Setup, cx: f32, cy: f32, r: f32, alpha: f32) {
    let style = Style::new(FG, alpha, GLOW);
    match setup {
        Setup::HotPotato => bomb(c, cx, cy, r, alpha),
        Setup::Roll(DieKind::PassThePot, _) => banknote(c, cx, cy, r, style),
        Setup::Roll(die, _) => die_solid(c, die, cx, cy, r, style),
    }
}

fn strokes<A: AssetStore>(c: &mut Ctx<A>, paths: &[&[P]], style: Style) {
    c.painter.stroke_paths(paths, LINE, style);
}

fn die_solid<A: AssetStore>(c: &mut Ctx<A>, die: DieKind, cx: f32, cy: f32, r: f32, style: Style) {
    let m = |p: P| at(cx, cy, r, p);
    match die {
        DieKind::D4 => {
            let (a, b, d) = (m((0.0, -0.95)), m((-0.95, 0.65)), m((0.95, 0.65)));
            let f = m((0.12, 0.2));
            strokes(c, &[&[a, b, d, a], &[a, f], &[b, f], &[d, f]], style);
        }
        DieKind::D6 => {
            let hex: [P; 6] = core::array::from_fn(|k| m(ngon(6, k, 0.95, 0.0)));
            let mid = m((0.0, 0.0));
            strokes(
                c,
                &[
                    &[hex[0], hex[1], hex[2], hex[3], hex[4], hex[5], hex[0]],
                    &[hex[1], mid, hex[5]],
                    &[mid, hex[3]],
                ],
                style,
            );
        }
        DieKind::D8 => {
            let (t, rt, b, l) = (m((0.0, -1.0)), m((0.9, 0.0)), m((0.0, 1.0)), m((-0.9, 0.0)));
            let f = m((0.0, 0.32));
            strokes(c, &[&[t, rt, b, l, t], &[l, f, rt], &[t, f], &[b, f]], style);
        }
        DieKind::D10 => kite(c, cx, cy, r, style),
        DieKind::D12 => {
            let outer: [P; 5] = core::array::from_fn(|k| m(ngon(5, k, 0.98, 0.0)));
            let inner: [P; 5] = core::array::from_fn(|k| m(ngon(5, k, 0.42, 0.0)));
            let ring_o = [outer[0], outer[1], outer[2], outer[3], outer[4], outer[0]];
            let ring_i = [inner[0], inner[1], inner[2], inner[3], inner[4], inner[0]];
            strokes(
                c,
                &[
                    &ring_o,
                    &ring_i,
                    &[inner[0], outer[0]],
                    &[inner[1], outer[1]],
                    &[inner[2], outer[2]],
                    &[inner[3], outer[3]],
                    &[inner[4], outer[4]],
                ],
                style,
            );
        }
        DieKind::D20 => {
            let hex: [P; 6] = core::array::from_fn(|k| m(ngon(6, k, 0.98, 0.0)));
            let tri: [P; 3] = core::array::from_fn(|k| m(ngon(3, k, 0.5, 0.0)));
            strokes(
                c,
                &[
                    &[hex[0], hex[1], hex[2], hex[3], hex[4], hex[5], hex[0]],
                    &[tri[0], tri[1], tri[2], tri[0]],
                    &[tri[0], hex[0]],
                    &[tri[1], hex[2]],
                    &[tri[2], hex[4]],
                    &[tri[0], hex[1]],
                    &[tri[0], hex[5]],
                    &[tri[1], hex[1]],
                    &[tri[1], hex[3]],
                    &[tri[2], hex[3]],
                    &[tri[2], hex[5]],
                ],
                style,
            );
        }
        // The tens die of a percentile roll, marked with a percent sign.
        _ => percentile(c, cx, cy, r, style),
    }
}

/// The percentile die: a d10 with its facet lines faint and a percent sign
/// on the front.
fn percentile<A: AssetStore>(c: &mut Ctx<A>, cx: f32, cy: f32, r: f32, style: Style) {
    let m = |p: P| at(cx, cy, r, p);
    let (t, rt, b, l) = (m((0.0, -1.0)), m((0.85, -0.12)), m((0.0, 1.0)), m((-0.85, -0.12)));
    let (p1, p2) = (m((-0.42, 0.34)), m((0.42, 0.34)));
    strokes(c, &[&[t, rt, b, l, t]], style);
    let faint = Style::new(FG, style.alpha * 0.45, 0.0);
    c.painter
        .stroke_paths(&[&[l, p1, b], &[rt, p2, b]], LINE * 0.7, faint);
    let (px, py, s) = (cx, cy + r * 0.02, r * 0.34);
    let ring = |dx: f32, dy: f32| (px + dx * s, py + dy * s);
    let (a, b) = (ring(-0.5, -0.62), ring(0.5, 0.62));
    let ro = s * 0.3;
    c.painter.stroke_arc(a.0, a.1, ro, 0.0, TAU, LINE * 0.8, style);
    c.painter.stroke_arc(b.0, b.1, ro, 0.0, TAU, LINE * 0.8, style);
    c.painter
        .stroke_polyline(&[ring(0.8, -1.0), ring(-0.8, 1.0)], LINE * 0.8, style);
}

/// The pentagonal trapezohedron of a d10, seen from the side.
fn kite<A: AssetStore>(c: &mut Ctx<A>, cx: f32, cy: f32, r: f32, style: Style) {
    let m = |p: P| at(cx, cy, r, p);
    let (t, rt, b, l) = (m((0.0, -1.0)), m((0.85, -0.12)), m((0.0, 1.0)), m((-0.85, -0.12)));
    let (p1, p2) = (m((-0.42, 0.34)), m((0.42, 0.34)));
    let mid = m((0.0, 0.12));
    strokes(
        c,
        &[
            &[t, rt, b, l, t],
            &[l, p1, b],
            &[rt, p2, b],
            &[t, mid],
            &[mid, p1],
            &[mid, p2],
        ],
        style,
    );
}

/// A banknote: border, inner frame, and a seal with a dollar sign.
fn banknote<A: AssetStore>(c: &mut Ctx<A>, cx: f32, cy: f32, r: f32, style: Style) {
    let (w, h) = (r * 2.1, r * 1.15);
    c.painter
        .stroke_rect(cx - w / 2.0, cy - h / 2.0, w, h, LINE, style);
    let (iw, ih) = (w - r * 0.45, h - r * 0.45);
    c.painter.stroke_rect(
        cx - iw / 2.0,
        cy - ih / 2.0,
        iw,
        ih,
        LINE * 0.6,
        Style::new(FG, style.alpha * 0.55, 0.0),
    );
    c.painter.stroke_arc(cx, cy, r * 0.4, 0.0, TAU, LINE * 0.8, style);
    let s = r * 0.24;
    let dollar = [
        (cx + s * 0.75, cy - s * 0.6),
        (cx + s * 0.3, cy - s * 0.95),
        (cx - s * 0.5, cy - s * 0.8),
        (cx - s * 0.55, cy - s * 0.2),
        (cx + s * 0.55, cy + s * 0.25),
        (cx + s * 0.5, cy + s * 0.85),
        (cx - s * 0.3, cy + s * 1.0),
        (cx - s * 0.75, cy + s * 0.65),
    ];
    let bar = [(cx, cy - s * 1.35), (cx, cy + s * 1.4)];
    c.painter
        .stroke_paths(&[&dollar, &bar], LINE * 0.7, Style::new(FG, style.alpha, 0.0));
    let (ox, oy) = (w / 2.0 - r * 0.32, h / 2.0 - r * 0.28);
    let dot = Style::new(FG, style.alpha, 0.0);
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        c.painter.fill_circle(cx + sx * ox, cy + sy * oy, LINE * 0.6, dot);
    }
}

/// A round bomb with a cap, a curling fuse and a spark.
fn bomb<A: AssetStore>(c: &mut Ctx<A>, cx: f32, cy: f32, r: f32, alpha: f32) {
    let (bx, by, br) = (cx - r * 0.1, cy + r * 0.2, r * 0.72);
    c.painter
        .fill_circle(bx, by, br, Style::new(FG, alpha * 0.22, 0.0));
    let style = Style::new(FG, alpha, GLOW);
    c.painter.stroke_arc(bx, by, br, 0.0, TAU, LINE, style);
    // The highlight, upper left.
    c.painter.stroke_arc(
        bx,
        by,
        br * 0.6,
        3.5,
        4.5,
        LINE * 0.8,
        Style::new(FG, alpha * 0.8, 0.0),
    );
    // Cap, then the fuse.
    let cap = (bx + br * 0.62, by - br * 0.72);
    c.painter.stroke_polyline(
        &[
            (cap.0 - br * 0.22, cap.1 + br * 0.24),
            (cap.0 + br * 0.2, cap.1 - br * 0.2),
        ],
        LINE * 2.4,
        style,
    );
    let fuse = [
        (cap.0 + br * 0.2, cap.1 - br * 0.2),
        (cap.0 + br * 0.4, cap.1 - br * 0.6),
        (cap.0 + br * 0.9, cap.1 - br * 0.55),
    ];
    c.painter.stroke_polyline(&fuse, LINE * 0.8, style);
    // The spark: a small star at the fuse's end.
    let (sx, sy) = (fuse[2].0 + br * 0.12, fuse[2].1 - br * 0.05);
    let l = br * 0.34;
    let d = l * 0.7;
    c.painter.stroke_paths(
        &[
            &[(sx - l, sy), (sx + l, sy)],
            &[(sx, sy - l), (sx, sy + l)],
            &[(sx - d, sy - d), (sx + d, sy + d)],
            &[(sx - d, sy + d), (sx + d, sy - d)],
        ],
        LINE * 0.7,
        style,
    );
}
