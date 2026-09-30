//! Drawing Sugar Run: one face of the maze, laid onto one screen by the
//! transform [`crate::maze::View::layers`] gives it.
//!
//! Walls are dim glowing lines, the crystals small dots, the sugar lumps
//! pulse, the runner chomps, and the ants are three beads with legs. Each
//! kind is one draw call, its shapes rasterized only where they are.

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
const DOT_R: f32 = 2.2;
const LUMP_R: f32 = 5.6;
const RUNNER_R: f32 = 8.2;
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
        ant(p, game, k, a, face, now, alpha);
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
    p.begin();
    for c in cells(face).filter(|c| game.has_dot(*c)) {
        let (x, y) = c.canvas();
        disc(p, x, y, DOT_R);
    }
    p.finish(Style::new(FG, 0.7 * alpha, 0.0));
    // The lumps breathe, so they stand out from the crystals.
    let pulse = 0.5 + 0.5 * cosf(now as f32 / 1000.0 * core::f32::consts::TAU * 1.4);
    p.begin();
    for c in cells(face).filter(|c| game.has_lump(*c)) {
        let (x, y) = c.canvas();
        disc(p, x, y, LUMP_R * (0.85 + 0.15 * pulse));
    }
    p.finish(Style::new(FG, (0.7 + 0.3 * pulse) * alpha, 9.0));
}

fn cells(face: Face) -> impl Iterator<Item = Cell> {
    (0..N * N).map(move |k| Cell::new(face, k % N, k / N))
}

fn disc(p: &mut Painter, x: f32, y: f32, r: f32) {
    p.add_shape((x - r, y - r, x + r, y + r), |px, py| {
        sqrtf((px - x) * (px - x) + (py - y) * (py - y)) - r
    });
}

/// The runner: a disc with a chomping mouth the way it's going. Caught, the
/// mouth opens all the way round and it's gone.
fn runner(p: &mut Painter, game: &Game, face: Face, now: u64, alpha: f32) {
    let Some((x, y, (ux, uy))) = game.runner.on(face) else {
        return;
    };
    let caught = match game.phase() {
        Phase::Caught { since } => Some((now.saturating_sub(since) as f32 / 1000.0 / 1.2).min(1.0)),
        Phase::Over { .. } => Some(1.0),
        _ => None,
    };
    let half = match caught {
        Some(u) => 0.35 + (core::f32::consts::PI - 0.35) * u,
        None if game.runner.moving => 0.08 + 0.62 * fabsf(sinf(game.chomp() * core::f32::consts::TAU * 2.6)),
        None => 0.45,
    };
    if half >= core::f32::consts::PI - 0.01 {
        return;
    }
    let (s, c) = (sinf(half), cosf(half));
    let r = RUNNER_R;
    p.begin();
    p.add_shape((x - r, y - r, x + r, y + r), |px, py| {
        let (dx, dy) = (px - x, py - y);
        let round = sqrtf(dx * dx + dy * dy) - r;
        let along = dx * ux + dy * uy;
        let across = fabsf(-dx * uy + dy * ux);
        // Negative inside the mouth's wedge.
        let mouth = across * c - along * s;
        round.max(-mouth)
    });
    p.finish(Style::new(FG, alpha, 7.0));
}

/// An ant: head, thorax and abdomen along the way it's going, three legs a
/// side that scurry as it walks, and feelers. Scared, it goes faint, and
/// flickers as the scare runs out.
fn ant(p: &mut Painter, game: &Game, k: usize, a: &maze::Ant, face: Face, now: u64, alpha: f32) {
    let w: &Walker = &a.w;
    let Some((x, y, (ux, uy))) = w.on(face) else {
        return;
    };
    let left = game.scared_left_ms(now);
    let flicker = left < maze::SCARE_WARN_MS && (now / 140) % 2 == 0;
    let (value, a_mul) = match (a.scared, flicker) {
        (true, false) => (FG, 0.35),
        (true, true) => (FG, 0.8),
        _ if matches!(a.state, AntState::Nest { .. }) => (ANT, 0.55),
        _ => (ANT, 1.0),
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
}

/// The runner as an icon: for the lives left, and the setup label.
pub fn draw_runner_icon(p: &mut Painter, x: f32, y: f32, r: f32, style: Style) {
    let (s, c) = (sinf(0.6), cosf(0.6));
    p.shape((x - r, y - r, x + r, y + r), style, |px, py| {
        let (dx, dy) = (px - x, py - y);
        let round = sqrtf(dx * dx + dy * dy) - r;
        let mouth = fabsf(dy) * c - dx * s;
        round.max(-mouth)
    });
}
