//! Drawing Sugar Run: one face of the maze, laid onto one screen by the
//! transform [`crate::maze::View::layers`] gives it.
//!
//! Walls are dim glowing lines, the crystals small twinkling diamonds, the
//! cube's scent a faint fading trail, the player a sugar cube that rocks as
//! it rolls, and the ants three beads with legs. Each kind is one draw call,
//! its shapes rasterized only where they are.

use heapless::Vec;
use libm::{cosf, fabsf, sinf, sqrtf};
use smokebomb_hal::Face;

use crate::display::FG;
use crate::gfx::{segment_distance, Painter, Style};
use crate::maze::{self, AntState, Cell, Game, Phase, Walker, CELL, HALF, N};

const WALL_W: f32 = 3.4;
/// Walls on a face's edge are drawn this far in, so the whole line shows
/// on each of the two screens rather than half of it on each.
const EDGE_IN: f32 = WALL_W / 2.0;
/// A crystal's half-diagonal, a scent speck's radius, and the sugar cube's
/// half-side and corner radius.
const CRYSTAL_R: f32 = 5.2;
const SCENT_R: f32 = 1.6;
const CUBE_H: f32 = 8.0;
const CUBE_ROUND: f32 = 2.2;
/// The ants' grey: dimmer than the runner, so they read apart on a grey
/// panel.
const ANT: u8 = 0xB4;

type Seg = ((f32, f32), (f32, f32));

/// Draw maze face `face` of `game` with the painter's current transform.
/// `alpha` dims it all (paused, over).
pub fn draw_face(p: &mut Painter, game: &Game, face: Face, now: u64, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    walls(p, game.maze(), face, alpha);
    sugar(p, game, face, now, alpha);
    for (k, a) in game.ants().iter().enumerate() {
        ant(p, k, a, face, now, alpha);
    }
    runner(p, game, face, now, alpha);
}

fn walls(p: &mut Painter, m: &maze::Maze, face: Face, alpha: f32) {
    let [right, left, down, up] = maze::ways(face);
    let lo = -HALF + EDGE_IN;
    let hi = HALF - EDGE_IN;
    let edge = |v: f32| v.clamp(lo, hi);
    let mut segs: Vec<Seg, { 2 * N * N + 2 * N }> = Vec::new();
    for j in 0..N {
        for i in 0..N {
            let c = Cell::new(face, i, j);
            let (x0, y0) = (-HALF + CELL * i as f32, -HALF + CELL * j as f32);
            let (x1, y1) = (x0 + CELL, y0 + CELL);
            if !m.open(c, right) {
                let _ = segs.push(((edge(x1), y0), (edge(x1), y1)));
            }
            if !m.open(c, down) {
                let _ = segs.push(((x0, edge(y1)), (x1, edge(y1))));
            }
            if i == 0 && !m.open(c, left) {
                let _ = segs.push(((edge(x0), y0), (edge(x0), y1)));
            }
            if j == 0 && !m.open(c, up) {
                let _ = segs.push(((x0, edge(y0)), (x1, edge(y0))));
            }
        }
    }
    let h = WALL_W / 2.0;
    p.begin();
    for (a, b) in segs {
        let bounds = (
            a.0.min(b.0) - h,
            a.1.min(b.1) - h,
            a.0.max(b.0) + h,
            a.1.max(b.1) + h,
        );
        p.add_shape(bounds, |x, y| segment_distance(x, y, a, b) - h);
    }
    p.finish(Style::new(FG, 0.62 * alpha, 6.0));
}

fn sugar(p: &mut Painter, game: &Game, face: Face, now: u64, alpha: f32) {
    // The scent: faint specks where the cube has been, fading as it does.
    // Ants follow it, so it's worth seeing.
    for c in cells(face) {
        let fresh = game.scent(c, now);
        if fresh > 0.05 {
            let (x, y) = c.canvas();
            p.begin();
            disc(p, x, y, SCENT_R);
            p.finish(Style::new(FG, 0.28 * fresh * alpha, 0.0));
        }
    }
    // Crystals: diamonds that twinkle, each in its own time.
    for c in game.crystals().iter().filter(|c| c.face() == face) {
        let (x, y) = c.canvas();
        let phase = c.0 as f32 * 0.7 + now as f32 / 1000.0 * core::f32::consts::TAU * 0.8;
        let twinkle = 0.5 + 0.5 * cosf(phase);
        let r = CRYSTAL_R * (0.85 + 0.15 * twinkle);
        p.begin();
        p.add_shape((x - r, y - r, x + r, y + r), |px, py| {
            (fabsf(px - x) + fabsf(py - y)) * core::f32::consts::FRAC_1_SQRT_2
                - r * core::f32::consts::FRAC_1_SQRT_2
        });
        p.finish(Style::new(FG, (0.65 + 0.35 * twinkle) * alpha, 8.0));
    }
}

fn cells(face: Face) -> impl Iterator<Item = Cell> {
    (0..N * N).map(move |k| Cell::new(face, k % N, k / N))
}

fn disc(p: &mut Painter, x: f32, y: f32, r: f32) {
    p.add_shape((x - r, y - r, x + r, y + r), |px, py| {
        sqrtf((px - x) * (px - x) + (py - y) * (py - y)) - r
    });
}

/// Signed distance to a rounded square of half-side `h` and corner radius
/// `round`, turned by `(cos, sin)`, from its centre to `(dx, dy)`.
fn cube_sdf(dx: f32, dy: f32, h: f32, round: f32, (c, s): (f32, f32)) -> f32 {
    let (lx, ly) = (dx * c + dy * s, -dx * s + dy * c);
    let (qx, qy) = (fabsf(lx) - (h - round), fabsf(ly) - (h - round));
    let outside = sqrtf(qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0));
    outside + qx.max(qy).min(0.0) - round
}

/// The player: a sugar cube seen from above, a bright rim round a dimmer
/// top with a glint, rocking a little as it rolls. Caught, it dissolves:
/// it shrinks and fades as the sugar bursts off it.
fn runner(p: &mut Painter, game: &Game, face: Face, now: u64, alpha: f32) {
    let Some((x, y, _)) = game.runner.on(face) else {
        return;
    };
    let gone = match game.phase() {
        Phase::Caught { since } => (now.saturating_sub(since) as f32 / 1000.0 / 1.2).min(1.0),
        Phase::Over { .. } => 1.0,
        _ => 0.0,
    };
    if gone >= 1.0 {
        return;
    }
    let rock = if game.runner.moving {
        0.22 * sinf(game.rolled() * core::f32::consts::TAU * 1.8)
    } else {
        0.0
    };
    draw_cube(p, x, y, CUBE_H * (1.0 - 0.7 * gone), rock, alpha * (1.0 - gone));
}

fn draw_cube(p: &mut Painter, x: f32, y: f32, h: f32, turn: f32, alpha: f32) {
    let (c, s) = (cosf(turn), sinf(turn));
    let e = h * 1.5;
    p.shape(
        (x - e, y - e, x + e, y + e),
        Style::new(FG, alpha, 7.0),
        |px, py| cube_sdf(px - x, py - y, h, CUBE_ROUND, (c, s)),
    );
    let inner = h * 0.62;
    p.shape(
        (x - e, y - e, x + e, y + e),
        Style::new(0x70, alpha, 0.0),
        |px, py| cube_sdf(px - x, py - y, inner, CUBE_ROUND * 0.6, (c, s)),
    );
    // The glint, up and to the left on the top.
    let (gx, gy) = (
        x + (-inner * 0.45) * c - (-inner * 0.45) * s,
        y + (-inner * 0.45) * s + (-inner * 0.45) * c,
    );
    let g = h * 0.2;
    p.shape(
        (gx - g, gy - g, gx + g, gy + g),
        Style::new(FG, alpha, 0.0),
        |px, py| sqrtf((px - gx) * (px - gx) + (py - gy) * (py - gy)) - g,
    );
}

/// An ant: head, thorax and abdomen along the way it's going, three legs a
/// side that scurry as it walks, and feelers. In the nest it's dim; dazed by
/// a burst it's faint and wobbles, with sugar sparks circling its head.
fn ant(p: &mut Painter, k: usize, a: &maze::Ant, face: Face, now: u64, alpha: f32) {
    let w: &Walker = &a.w;
    let Some((x, y, (ux, uy))) = w.on(face) else {
        return;
    };
    let dazed = matches!(a.state, AntState::Dazed { .. });
    let a_mul = match a.state {
        AntState::Nest => 0.55,
        AntState::Dazed { .. } => 0.45,
        AntState::Out => 1.0,
    };
    let value = ANT;
    let (ux, uy) = if dazed {
        // A wobble about where it stands.
        let t = sinf(now as f32 / 1000.0 * core::f32::consts::TAU * 3.0) * 0.35;
        (ux * cosf(t) - uy * sinf(t), ux * sinf(t) + uy * cosf(t))
    } else {
        (ux, uy)
    };
    let (vx, vy) = (-uy, ux);
    let at = |f: f32, s: f32| (x + ux * f + vx * s, y + uy * f + vy * s);
    let step = if w.moving {
        sinf((w.t + k as f32 * 0.37) * core::f32::consts::TAU * 2.0) * 1.6
    } else {
        0.0
    };
    p.begin();
    for (f, r) in [(5.6, 3.0), (0.6, 2.2), (-4.8, 4.0)] {
        let (cx, cy) = at(f, 0.0);
        disc(p, cx, cy, r);
    }
    let mut lines: Vec<Seg, 8> = Vec::new();
    for (i, f) in [3.0f32, 0.6, -1.8].into_iter().enumerate() {
        let swing = if i % 2 == 0 { step } else { -step };
        for s in [-1.0f32, 1.0] {
            let _ = lines.push((at(f, 0.0), at(f + swing * s + (1.0 - i as f32) * 1.4, 6.4 * s)));
        }
    }
    for s in [-1.0f32, 1.0] {
        let _ = lines.push((at(7.4, 1.0 * s), at(10.4, 3.4 * s)));
    }
    let h = 0.75;
    for (a0, b0) in lines {
        let bounds = (
            a0.0.min(b0.0) - h,
            a0.1.min(b0.1) - h,
            a0.0.max(b0.0) + h,
            a0.1.max(b0.1) + h,
        );
        p.add_shape(bounds, |px, py| segment_distance(px, py, a0, b0) - h);
    }
    p.finish(Style::new(value, a_mul * alpha, 0.0));
    if dazed {
        let (hx, hy) = at(5.6, 0.0);
        p.begin();
        for i in 0..3 {
            let t = now as f32 / 1000.0 * core::f32::consts::TAU * 1.2 + i as f32 * 2.09;
            disc(p, hx + 6.0 * cosf(t), hy + 6.0 * sinf(t), 1.3);
        }
        p.finish(Style::new(FG, 0.9 * alpha, 3.0));
    }
}

/// The sugar cube as an icon: for the lives left, and the setup label. `r`
/// is its half-side.
pub fn draw_cube_icon(p: &mut Painter, x: f32, y: f32, r: f32, style: Style) {
    draw_cube(p, x, y, r, 0.0, style.alpha);
}

/// A crystal as an icon, `r` its half-diagonal.
pub fn draw_crystal_icon(p: &mut Painter, x: f32, y: f32, r: f32, style: Style) {
    p.shape((x - r, y - r, x + r, y + r), style, |px, py| {
        (fabsf(px - x) + fabsf(py - y) - r) * core::f32::consts::FRAC_1_SQRT_2
    });
}
