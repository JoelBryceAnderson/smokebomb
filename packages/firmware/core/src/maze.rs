//! Sugar Run: a maze over the whole cube.
//!
//! Every face is an 8×8 grid of cells, and the corridors carry on over the
//! edges, so the maze has no border: it is the surface of the die. You are a
//! sugar cube hunting sugar crystals round it, and ants are after you. Tilt
//! the die to steer; the cube goes downhill, turning at the next opening
//! like a joystick held over.
//!
//! A few crystals lie about at a time; take one and another turns up
//! somewhere else, usually on another face, so the side screens are where
//! you look for the next. You leave a scent that fades, and an ant that
//! crosses it follows it to you; close by, it rushes you. More ants hatch
//! from the nest as the score climbs. A shake bursts sugar: ants near you
//! are dazed for a moment. Bursts are earned by collecting crystals.
//!
//! The cube always stays on the top screen. When it crosses an edge the map
//! rolls a quarter turn over the die, bringing the face it ran onto up
//! ([`View`]). A cube's surface can only be turned onto itself in quarter
//! turns, so the map rolls a face at a time rather than scrolling. The four
//! side screens show the faces next to the cube's, so you see ants coming;
//! the face underneath is the far side of the world.
//!
//! ```text
//!   Ready ── shake ──▶ Playing ── caught, lives left ──▶ Caught ──▶ Playing
//!                       │  ▲
//!           set down ── │  └── tilt          caught, no lives ──▶ Over ── shake ──▶ Playing
//!                       ▼
//!                     Paused
//! ```
//!
//! Like [`crate::potato`] the game is pure: the firmware passes in the time,
//! the way the die is tipped and a seed from the TRNG, and turns the
//! [`RunEvent`]s it gets back into haptics and sugar.
//!
//! Positions are doubled integer coordinates on a cube of side 16: a cell
//! centre on face `n` is `8·n + (2i − 7)·x − (2j − 7)·y` for the face's
//! drawing axes ([`BASES`]), with `i` its column and `j` its row down the
//! face. A step is 2 units; stepping off a face turns the corner onto the
//! next.

use heapless::Vec;
use smokebomb_hal::{Face, FACE_COUNT};

use crate::gfx::Transform;
use crate::orientation::{Quarter, BASES};

/// Cells along a face's side.
pub const N: usize = 8;
/// Cells on the whole die.
pub const CELLS: usize = FACE_COUNT * N * N;
/// Doubled coordinate of the edge cells' centres, in the face's plane.
const EDGE: i32 = N as i32 - 1;
/// Doubled distance from the die's centre to a face.
const OUT: i32 = N as i32;

/// After the maze has no dead ends, this share of its walls come down, so
/// the corridors run long and there is always a way round an ant.
const LOOPS_PCT: u32 = 24;

/// The most ants a game can have (the Ants page).
pub const MAX_ANTS: u8 = 4;
pub const MIN_ANTS: u8 = 1;
pub const DEFAULT_ANTS: u8 = 3;
pub const LIVES: u8 = 3;

/// Cube and ant speeds, cells a second. Ants get quicker with the score,
/// up to the most they reach.
const RUNNER_SPEED: f32 = 4.2;
const ANT_SPEED: f32 = 3.0;
const ANT_SPEED_PER_CRYSTAL: f32 = 0.05;
const ANT_MAX: f32 = 4.6;
/// Crystals lying about at once.
pub const CRYSTALS: usize = 6;
/// The first ant hatches this long into a life, the next no sooner than
/// this after the last; one more may be out for every few crystals.
const HATCH_FIRST_MS: u64 = 2_000;
const HATCH_GAP_MS: u64 = 3_000;
const CRYSTALS_PER_ANT: u32 = 4;
/// How long the cube's scent lasts, and how close (doubled units, squared)
/// an ant must be to smell the cube itself and rush it.
pub const SCENT_MS: u64 = 8_000;
const RUSH: i32 = 6 * 6;
/// Bursts: one to start, one more for every few crystals, up to a few. A
/// burst dazes ants within this (doubled units, squared) for this long.
const BURSTS_START: u8 = 1;
pub const BURSTS_MAX: u8 = 3;
const CRYSTALS_PER_BURST: u32 = 5;
const BURST_REACH: f32 = 10.0 * 10.0;
pub const DAZE_MS: u64 = 3_500;
/// How close (doubled units) an ant must come to catch the cube.
const CATCH: f32 = 1.2;
/// How long the caught cube dissolves before play goes on.
pub const CAUGHT_MS: u64 = 1_800;

type V3 = [i32; 3];

fn dot(a: V3, b: V3) -> i32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: i32) -> V3 {
    a.map(|c| c * k)
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn ivec(v: [f32; 3]) -> V3 {
    v.map(|c| c as i32)
}

/// A face's outward normal, as a direction: faces double as the six ways
/// along the die's axes.
pub fn axis(face: Face) -> V3 {
    ivec(BASES[face.index()].n)
}

/// The face whose normal is the axis direction `v`.
fn face_along(v: V3) -> Face {
    match v {
        [x, _, _] if x > 0 => Face::PosX,
        [x, _, _] if x < 0 => Face::NegX,
        [_, y, _] if y > 0 => Face::PosY,
        [_, y, _] if y < 0 => Face::NegY,
        [_, _, z] if z > 0 => Face::PosZ,
        _ => Face::NegZ,
    }
}

/// The four ways along `face`: canvas right, left, down and up.
pub fn ways(face: Face) -> [Face; 4] {
    let b = &BASES[face.index()];
    let (x, y) = (ivec(b.x), ivec(b.y));
    [
        face_along(x),
        face_along(scale(x, -1)),
        face_along(scale(y, -1)),
        face_along(y),
    ]
}

/// Which of `face`'s four ways `d` is, if it lies along the face.
fn slot(face: Face, d: Face) -> Option<usize> {
    ways(face).iter().position(|w| *w == d)
}

/// `d` as a canvas direction on `face` (x right, y down).
pub fn canvas_dir(face: Face, d: Face) -> (f32, f32) {
    let b = &BASES[face.index()];
    let v = axis(d);
    (dot(v, ivec(b.x)) as f32, -dot(v, ivec(b.y)) as f32)
}

/// One cell of the maze.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell(pub u16);

impl Cell {
    /// Column `i` and row `j` (down the face) of `face`.
    pub const fn new(face: Face, i: usize, j: usize) -> Self {
        Self((face.index() * N * N + j * N + i) as u16)
    }

    pub fn face(self) -> Face {
        Face::ALL[self.0 as usize / (N * N)]
    }

    /// Column and row on its face.
    pub fn ij(self) -> (usize, usize) {
        let k = self.0 as usize % (N * N);
        (k % N, k / N)
    }

    const fn index(self) -> usize {
        self.0 as usize
    }

    /// The centre in doubled die coordinates.
    pub fn pos(self) -> V3 {
        let b = &BASES[self.face().index()];
        let (i, j) = self.ij();
        let u = 2 * i as i32 - EDGE;
        let v = 2 * j as i32 - EDGE;
        add(
            add(scale(ivec(b.n), OUT), scale(ivec(b.x), u)),
            scale(ivec(b.y), -v),
        )
    }

    fn at(face: Face, p: V3) -> Self {
        let b = &BASES[face.index()];
        let i = (dot(p, ivec(b.x)) + EDGE) / 2;
        let j = (EDGE - dot(p, ivec(b.y))) / 2;
        Self::new(face, i as usize, j as usize)
    }

    /// The centre in canvas units on its face, where the lit area is ±83
    /// (as [`crate::screens::ACTIVE`]).
    pub fn canvas(self) -> (f32, f32) {
        let (i, j) = self.ij();
        (-HALF + CELL * (i as f32 + 0.5), -HALF + CELL * (j as f32 + 0.5))
    }

    /// The next cell along `d` (a way along this cell's face), and the way
    /// you are going when you get there: over an edge, the corner turns it.
    pub fn step(self, d: Face) -> (Cell, Face) {
        let n = axis(self.face());
        let dv = axis(d);
        let p = self.pos();
        let q = add(p, scale(dv, 2));
        if dot(q, dv) <= EDGE {
            (Cell::at(self.face(), q), d)
        } else {
            (Cell::at(d, add(add(p, scale(n, -1)), dv)), self.face().opposite())
        }
    }
}

/// Half a face and one cell, in canvas units.
pub const HALF: f32 = 83.0;
pub const CELL: f32 = 2.0 * HALF / N as f32;

/// The walls: which of each cell's four ways are open.
#[derive(Clone)]
pub struct Maze {
    open: [u8; CELLS],
}

impl Default for Maze {
    fn default() -> Self {
        Self::new()
    }
}

impl Maze {
    /// Walls everywhere.
    pub const fn new() -> Self {
        Self { open: [0; CELLS] }
    }

    /// Can you go from `c` along `d`? Never across a face (`d` along its
    /// normal).
    pub fn open(&self, c: Cell, d: Face) -> bool {
        slot(c.face(), d).is_some_and(|s| self.open[c.index()] & (1 << s) != 0)
    }

    fn carve(&mut self, c: Cell, d: Face) {
        let (next, arrive) = c.step(d);
        for (cell, way) in [(c, d), (next, arrive.opposite())] {
            if let Some(s) = slot(cell.face(), way) {
                self.open[cell.index()] |= 1 << s;
            }
        }
    }

    /// How many ways out of `c` are open.
    pub fn exits(&self, c: Cell) -> u32 {
        self.open[c.index()].count_ones()
    }

    /// A new maze: a random spanning tree (so every cell can reach every
    /// other), then no dead ends, then some extra loops so there is always
    /// a way round an ant.
    pub fn generate(&mut self, rng: &mut Rng) {
        self.open = [0; CELLS];
        let mut seen = [false; CELLS];
        let mut stack: Vec<u16, CELLS> = Vec::new();
        seen[0] = true;
        let _ = stack.push(0);
        while let Some(&top) = stack.last() {
            let c = Cell(top);
            let mut fresh: Vec<Face, 4> = Vec::new();
            for d in ways(c.face()) {
                if !seen[c.step(d).0.index()] {
                    let _ = fresh.push(d);
                }
            }
            if fresh.is_empty() {
                stack.pop();
                continue;
            }
            let d = fresh[rng.below(fresh.len() as u32) as usize];
            let next = c.step(d).0;
            self.carve(c, d);
            seen[next.index()] = true;
            let _ = stack.push(next.0);
        }
        // Braid: open a dead end onto a neighbour, another dead end if it
        // has one.
        for k in 0..CELLS as u16 {
            let c = Cell(k);
            if self.exits(c) != 1 {
                continue;
            }
            let mut shut: Vec<Face, 4> = Vec::new();
            for d in ways(c.face()) {
                if !self.open(c, d) {
                    let _ = shut.push(d);
                }
            }
            let dead = shut.iter().copied().find(|d| self.exits(c.step(*d).0) == 1);
            let d = dead.unwrap_or_else(|| shut[rng.below(shut.len() as u32) as usize]);
            self.carve(c, d);
        }
        // Loops.
        for k in 0..CELLS as u16 {
            let c = Cell(k);
            for d in ways(c.face()) {
                if !self.open(c, d) && rng.below(100) < LOOPS_PCT {
                    self.carve(c, d);
                }
            }
        }
    }
}

/// xorshift32: the game's own random numbers, seeded from the TRNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(u32);

impl Rng {
    pub const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9e37_79b9 } else { seed })
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform in `0..n` (n > 0), near enough for a game.
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }
}

/// Something that walks the maze: in `cell`, `t` of the way (0–1) to the
/// next one along `dir`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    pub cell: Cell,
    /// Always a way along `cell`'s face.
    pub dir: Face,
    pub t: f32,
    pub moving: bool,
}

impl Walker {
    const fn at(cell: Cell, dir: Face) -> Self {
        Self {
            cell,
            dir,
            t: 0.0,
            moving: false,
        }
    }

    /// Where it is, in doubled die coordinates. Crossing an edge it cuts
    /// the corner a little, which is fine for telling whether two meet.
    pub fn pos(&self) -> [f32; 3] {
        let p = self.cell.pos();
        let d = axis(self.dir);
        [0, 1, 2].map(|k| p[k] as f32 + d[k] as f32 * 2.0 * self.t)
    }

    /// The face it's on: past halfway to a cell over an edge, that cell's.
    pub fn face(&self) -> Face {
        if self.t >= 0.5 {
            self.ahead().0.face()
        } else {
            self.cell.face()
        }
    }

    /// The next cell and the way it arrives there.
    pub fn ahead(&self) -> (Cell, Face) {
        self.cell.step(self.dir)
    }

    /// Where to draw it on `face`, in canvas units, and the way it faces
    /// there: on its own face, or on the next as it crosses over.
    pub fn on(&self, face: Face) -> Option<(f32, f32, (f32, f32))> {
        let (next, arrive) = self.ahead();
        if self.cell.face() == face {
            let (x, y) = self.cell.canvas();
            let d = canvas_dir(face, self.dir);
            Some((x + d.0 * CELL * self.t, y + d.1 * CELL * self.t, d))
        } else if next.face() == face && self.t > 0.0 {
            let (x, y) = next.canvas();
            let d = canvas_dir(face, arrive);
            let back = 1.0 - self.t;
            Some((x - d.0 * CELL * back, y - d.1 * CELL * back, d))
        } else {
            None
        }
    }

    /// Turn round where it stands.
    fn reverse(&mut self) {
        if self.moving && self.t > 0.0 {
            let (next, arrive) = self.ahead();
            self.cell = next;
            self.dir = arrive.opposite();
            self.t = 1.0 - self.t;
        } else {
            self.dir = self.dir.opposite();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AntState {
    /// In the nest, not hatched yet.
    Nest,
    Out,
    /// Caught in a burst of sugar: stopped, and harmless, until then.
    Dazed {
        until: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ant {
    pub w: Walker,
    pub state: AntState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing started: the die shows its setup label.
    Ready,
    Playing,
    /// Set down: the maze waits, dimmed, until the die is tilted again.
    Paused,
    /// An ant got the cube.
    Caught {
        since: u64,
    },
    /// No lives left: the score shows until the next shake.
    Over {
        since: u64,
    },
}

/// What happened in a step, for the firmware to feel and show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEvent {
    Started,
    Crystal,
    /// A crystal that also earned a burst.
    Earned,
    Burst,
    Caught,
    GameOver,
}

pub type RunEvents = Vec<RunEvent, 8>;

/// Where the cube starts, and where the ants' nest is: the far side.
pub const START: Cell = Cell::new(Face::PosY, 3, 4);
const NEST: [Cell; MAX_ANTS as usize] = [
    Cell::new(Face::NegY, 3, 3),
    Cell::new(Face::NegY, 4, 4),
    Cell::new(Face::NegY, 4, 3),
    Cell::new(Face::NegY, 3, 4),
];

pub struct Game {
    maze: Maze,
    /// The crystals lying about.
    crystals: Vec<Cell, CRYSTALS>,
    /// When the cube last passed each cell (ms since the game began, plus
    /// one; 0 for never): its scent.
    scent: [u32; CELLS],
    pub runner: Walker,
    ants: [Ant; MAX_ANTS as usize],
    ant_count: u8,
    phase: Phase,
    /// Crystals collected.
    score: u32,
    lives: u8,
    bursts: u8,
    /// When this life began, and when an ant last hatched.
    round_start: u64,
    last_hatch: u64,
    /// When the game began: scent is kept relative to it.
    epoch: u64,
    rng: Rng,
    /// Seconds the cube has been moving, for its wobble.
    rolled: f32,
}

impl Game {
    pub fn new(ants: u8) -> Self {
        Self {
            maze: Maze::new(),
            crystals: Vec::new(),
            scent: [0; CELLS],
            runner: Walker::at(START, Face::PosX),
            ants: [Ant {
                w: Walker::at(NEST[0], Face::PosX),
                state: AntState::Nest,
            }; MAX_ANTS as usize],
            ant_count: ants.clamp(MIN_ANTS, MAX_ANTS),
            phase: Phase::Ready,
            score: 0,
            lives: LIVES,
            bursts: BURSTS_START,
            round_start: 0,
            last_hatch: 0,
            epoch: 0,
            rng: Rng::new(1),
            rolled: 0.0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn maze(&self) -> &Maze {
        &self.maze
    }

    /// Crystals collected.
    pub fn score(&self) -> u32 {
        self.score
    }

    pub fn lives(&self) -> u8 {
        self.lives
    }

    /// Bursts in hand.
    pub fn bursts(&self) -> u8 {
        self.bursts
    }

    /// The most ants that may be out.
    pub fn ant_count(&self) -> u8 {
        self.ant_count
    }

    pub fn ants(&self) -> &[Ant] {
        &self.ants[..self.ant_count as usize]
    }

    pub fn crystals(&self) -> &[Cell] {
        &self.crystals
    }

    pub fn has_crystal(&self, c: Cell) -> bool {
        self.crystals.contains(&c)
    }

    /// How fresh the cube's scent on `c` is at `now`: 1 just passed, 0 gone.
    pub fn scent(&self, c: Cell, now: u64) -> f32 {
        match self.scent[c.index()] {
            0 => 0.0,
            t => {
                let age = now.saturating_sub(self.epoch + t as u64 - 1);
                (1.0 - age as f32 / SCENT_MS as f32).max(0.0)
            }
        }
    }

    /// Seconds the cube has been moving, for its wobble.
    pub fn rolled(&self) -> f32 {
        self.rolled
    }

    /// A game is under way (not waiting to start, and not over).
    pub fn in_play(&self) -> bool {
        !matches!(self.phase, Phase::Ready | Phase::Over { .. })
    }

    /// The maze is moving: nothing should interrupt it.
    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Playing | Phase::Caught { .. })
    }

    /// Start a game, or a new one once it's over (the firmware does this
    /// when a shake settles).
    pub fn start(&mut self, now: u64, seed: u32) -> RunEvents {
        let mut out = RunEvents::new();
        if matches!(self.phase, Phase::Ready | Phase::Over { .. }) {
            self.rng = Rng::new(seed);
            self.score = 0;
            self.lives = LIVES;
            self.bursts = BURSTS_START;
            self.epoch = now;
            self.scent = [0; CELLS];
            self.maze.generate(&mut self.rng);
            self.new_round(now);
            self.crystals.clear();
            while !self.crystals.is_full() {
                self.drop_crystal();
            }
            let _ = out.push(RunEvent::Started);
        }
        out
    }

    /// The die was set down: wait.
    pub fn pause(&mut self) {
        if self.phase == Phase::Playing {
            self.phase = Phase::Paused;
        }
    }

    /// Picked up and tilted: carry on. The ants' and the scent's clocks
    /// don't count the pause.
    pub fn resume(&mut self, paused_ms: u64) {
        if self.phase == Phase::Paused {
            self.phase = Phase::Playing;
            self.round_start += paused_ms;
            self.last_hatch += paused_ms;
            self.epoch += paused_ms;
            for a in &mut self.ants {
                if let AntState::Dazed { until } = &mut a.state {
                    *until += paused_ms;
                }
            }
        }
    }

    /// Back to the start, as when the mode is left.
    pub fn reset(&mut self) {
        *self = Self::new(self.ant_count);
    }

    /// A shake mid-run: burst sugar, if there's a burst in hand. Ants near
    /// the cube are dazed.
    pub fn burst(&mut self, now: u64) -> RunEvents {
        let mut out = RunEvents::new();
        if self.phase != Phase::Playing || self.bursts == 0 {
            return out;
        }
        self.bursts -= 1;
        let p = self.runner.pos();
        for a in &mut self.ants {
            if matches!(a.state, AntState::Out | AntState::Dazed { .. }) && dist2(p, a.w.pos()) < BURST_REACH
            {
                a.state = AntState::Dazed { until: now + DAZE_MS };
            }
        }
        let _ = out.push(RunEvent::Burst);
        out
    }

    /// Put a crystal down where there isn't one, away from the cube: on
    /// another face if a few tries find one.
    fn drop_crystal(&mut self) {
        let here = self.runner.cell;
        for tries in 0..16 {
            let c = Cell(self.rng.below(CELLS as u32) as u16);
            let taken = self.crystals.contains(&c) || c == here || NEST.contains(&c);
            if taken || (tries < 8 && c.face() == here.face()) {
                continue;
            }
            let _ = self.crystals.push(c);
            return;
        }
    }

    /// Everyone back to the start for a new life: the ants in the nest to
    /// hatch again, the crystals where they were.
    fn new_round(&mut self, now: u64) {
        let start_dir = ways(START.face())
            .into_iter()
            .find(|d| self.maze.open(START, *d))
            .unwrap_or(Face::PosX);
        self.runner = Walker::at(START, start_dir);
        for (k, a) in self.ants.iter_mut().enumerate() {
            a.w = Walker::at(NEST[k], ways(NEST[k].face())[k % 4]);
            a.state = AntState::Nest;
        }
        self.round_start = now;
        self.last_hatch = 0;
        self.phase = Phase::Playing;
    }

    fn ant_speed(&self) -> f32 {
        (ANT_SPEED + ANT_SPEED_PER_CRYSTAL * self.score as f32).min(ANT_MAX)
    }

    /// Advance by `dt` seconds. `want` is the way the die is tipped, as a
    /// way through the maze (it may not lie along the cube's face while the
    /// map is still rolling round to it; then it waits).
    pub fn step(&mut self, now: u64, dt: f32, want: Option<Face>) -> RunEvents {
        let mut out = RunEvents::new();
        match self.phase {
            Phase::Playing => {}
            Phase::Caught { since } if now.saturating_sub(since) >= CAUGHT_MS => {
                if self.lives == 0 {
                    self.phase = Phase::Over { since: now };
                    let _ = out.push(RunEvent::GameOver);
                } else {
                    self.new_round(now);
                }
                return out;
            }
            _ => return out,
        }
        let dt = dt.clamp(0.0, 0.1);
        self.move_runner(now, dt, want, &mut out);
        self.hatch(now);
        for k in 0..self.ant_count as usize {
            self.move_ant(k, now, dt);
        }
        self.meet(now, &mut out);
        out
    }

    /// Let another ant out of the nest when it's due: the first a moment
    /// into a life, then one more for every few crystals, spaced out.
    fn hatch(&mut self, now: u64) {
        let out = self.ants().iter().filter(|a| a.state != AntState::Nest).count() as u32;
        let allowed = (1 + self.score / CRYSTALS_PER_ANT).min(self.ant_count as u32);
        let due = if out == 0 {
            now.saturating_sub(self.round_start) >= HATCH_FIRST_MS
        } else {
            now.saturating_sub(self.last_hatch) >= HATCH_GAP_MS
        };
        if out < allowed && due {
            if let Some(a) = self.ants[..self.ant_count as usize]
                .iter_mut()
                .find(|a| a.state == AntState::Nest)
            {
                a.state = AntState::Out;
                a.w.moving = true;
                self.last_hatch = now;
            }
        }
    }

    fn move_runner(&mut self, now: u64, dt: f32, want: Option<Face>, out: &mut RunEvents) {
        let r = &mut self.runner;
        if let Some(w) = want {
            if r.moving && w == r.dir.opposite() {
                r.reverse();
            } else if !r.moving && self.maze.open(r.cell, w) {
                r.dir = w;
            }
        }
        if r.t == 0.0 && !self.maze.open(r.cell, r.dir) {
            r.moving = false;
        }
        if !r.moving {
            r.moving = self.maze.open(r.cell, r.dir);
            if !r.moving {
                return;
            }
        }
        self.rolled += dt;
        r.t += RUNNER_SPEED * dt;
        while r.t >= 1.0 {
            let (next, arrive) = r.ahead();
            self.scent[r.cell.index()] = now.saturating_sub(self.epoch) as u32 + 1;
            r.cell = next;
            r.dir = arrive;
            r.t -= 1.0;
            if let Some(i) = self.crystals.iter().position(|c| *c == next) {
                self.crystals.swap_remove(i);
                self.score += 1;
                let earned = self.score % CRYSTALS_PER_BURST == 0 && self.bursts < BURSTS_MAX;
                if earned {
                    self.bursts += 1;
                }
                let _ = out.push(if earned {
                    RunEvent::Earned
                } else {
                    RunEvent::Crystal
                });
            }
            if let Some(w) = want.filter(|w| self.maze.open(next, *w)) {
                r.dir = w;
            } else if !self.maze.open(next, r.dir) {
                r.t = 0.0;
                r.moving = false;
            }
        }
        // Another crystal turns up for each one taken.
        while !self.crystals.is_full() {
            self.drop_crystal();
        }
    }

    fn move_ant(&mut self, k: usize, now: u64, dt: f32) {
        let speed = self.ant_speed();
        let runner = self.runner.cell.pos();
        match self.ants[k].state {
            AntState::Nest => return,
            AntState::Dazed { until } if now < until => return,
            AntState::Dazed { .. } => self.ants[k].state = AntState::Out,
            AntState::Out => {}
        }
        let mut w = self.ants[k].w;
        w.t += speed * dt;
        // Just hatched, or blocked: pick a way now.
        if !self.maze.open(w.cell, w.dir) && w.t < 1.0 {
            w.t = 0.0;
            w.dir = self.choose(w.cell, w.dir, runner, now, true);
        }
        while w.t >= 1.0 {
            let (next, arrive) = w.ahead();
            w.cell = next;
            w.t -= 1.0;
            w.dir = self.choose(next, arrive, runner, now, false);
        }
        self.ants[k].w = w;
    }

    /// An ant at a cell centre, arriving along `arrive`, picks its way:
    /// never straight back unless it must (or `fresh`, just hatched). Close
    /// to the cube it rushes it; on the cube's trail it follows the scent
    /// the way it's fresher; otherwise it wanders.
    fn choose(&mut self, cell: Cell, arrive: Face, runner: V3, now: u64, fresh: bool) -> Face {
        let mut options: Vec<Face, 4> = Vec::new();
        for d in ways(cell.face()) {
            if self.maze.open(cell, d) && (fresh || d != arrive.opposite()) {
                let _ = options.push(d);
            }
        }
        if options.is_empty() {
            return arrive.opposite();
        }
        let gap = add(cell.pos(), scale(runner, -1));
        if dot(gap, gap) <= RUSH {
            // Nearest by direction from the die's centre, which works over
            // the edges too.
            let toward = |d: &Face| {
                let p = cell.step(*d).0.pos();
                dot(p, runner) as f32 / libm::sqrtf(dot(p, p) as f32)
            };
            if let Some(d) = options
                .iter()
                .copied()
                .max_by(|a, b| toward(a).total_cmp(&toward(b)))
            {
                return d;
            }
        }
        let here = self.scent(cell, now);
        let trail = options
            .iter()
            .copied()
            .map(|d| (d, self.scent(cell.step(d).0, now)))
            .filter(|(_, s)| *s > 0.0 && *s > here)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((d, _)) = trail {
            return d;
        }
        options[self.rng.below(options.len() as u32) as usize]
    }

    /// Did the cube meet an ant that's out (not dazed)? Then it's caught.
    fn meet(&mut self, now: u64, out: &mut RunEvents) {
        let p = self.runner.pos();
        let caught = self
            .ants()
            .iter()
            .any(|a| a.state == AntState::Out && dist2(p, a.w.pos()) < CATCH * CATCH);
        if caught {
            self.lives = self.lives.saturating_sub(1);
            self.runner.moving = false;
            self.phase = Phase::Caught { since: now };
            let _ = out.push(RunEvent::Caught);
        }
    }
}

fn dist2(p: [f32; 3], q: [f32; 3]) -> f32 {
    (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum()
}

// ---------- the map on the die ----------

/// A turn of the maze against the die, as a signed permutation matrix:
/// maze direction = `M · die direction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Turn([[i8; 3]; 3]);

impl Turn {
    pub const IDENTITY: Turn = Turn([[1, 0, 0], [0, 1, 0], [0, 0, 1]]);

    pub fn apply(&self, v: V3) -> V3 {
        let m = &self.0;
        [0, 1, 2].map(|r| (0..3).map(|c| m[r][c] as i32 * v[c]).sum())
    }

    /// The inverse turn, applied: maze direction → die direction.
    pub fn unapply(&self, v: V3) -> V3 {
        let m = &self.0;
        [0, 1, 2].map(|c| (0..3).map(|r| m[r][c] as i32 * v[r]).sum())
    }

    /// `self` after `q`: `(self · q) v = self (q v)`.
    fn then(&self, q: &Turn) -> Turn {
        let mut out = [[0i8; 3]; 3];
        for (r, row) in out.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| self.0[r][k] * q.0[k][c]).sum();
            }
        }
        Turn(out)
    }

    /// A quarter turn about the axis direction `a`: `v ↦ (a·v)a + a × v`.
    fn quarter_about(a: V3) -> Turn {
        let col = |e: V3| add(scale(a, dot(a, e)), cross(a, e));
        let cols = [col([1, 0, 0]), col([0, 1, 0]), col([0, 0, 1])];
        Turn([0, 1, 2].map(|r| [0, 1, 2].map(|c| cols[c][r] as i8)))
    }

    /// The maze face shown on the die's `face`.
    pub fn maze_face(&self, face: Face) -> Face {
        face_along(self.apply(axis(face)))
    }

    /// How the maze face's canvas sits on the die's `face` (a quarter
    /// turn, as the text orientation is).
    pub fn quarter(&self, face: Face) -> Quarter {
        let (cos, sin) = self.cos_sin(face);
        match (cos, sin) {
            (1, _) => Quarter::R0,
            (_, 1) => Quarter::R90,
            (-1, _) => Quarter::R180,
            _ => Quarter::R270,
        }
    }

    fn cos_sin(&self, face: Face) -> (i32, i32) {
        let f = &BASES[face.index()];
        let g = &BASES[self.maze_face(face).index()];
        let gx = ivec(g.x);
        (dot(gx, self.apply(ivec(f.x))), -dot(gx, self.apply(ivec(f.y))))
    }
}

/// How long the map takes to roll a quarter turn.
pub const ROLL_MS: u64 = 280;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Roll {
    from: Turn,
    /// The die axis it turns about.
    about: V3,
    start: u64,
}

/// Which part of the maze each screen shows, keeping the runner's face on
/// top.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View {
    turn: Turn,
    roll: Option<Roll>,
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

impl View {
    pub const fn new() -> Self {
        Self {
            turn: Turn::IDENTITY,
            roll: None,
        }
    }

    /// Where the map is heading (settled, or once the roll ends).
    pub fn turn(&self) -> Turn {
        self.turn
    }

    pub fn rolling(&self) -> bool {
        self.roll.is_some()
    }

    /// The quarter turn that brings maze face `target` a step toward the
    /// die's `up` face, if it isn't there.
    fn step_toward(turn: &Turn, up: Face, target: Face) -> Option<V3> {
        let d = turn.unapply(axis(target));
        let n = axis(up);
        if d == n {
            return None;
        }
        // On the far side: roll it round to a side first.
        let side = if d == scale(n, -1) { axis(ways(up)[0]) } else { d };
        Some(cross(n, side))
    }

    /// Keep maze face `target` (the runner's) on the die's `up` face,
    /// rolling the map there a quarter turn at a time.
    pub fn update(&mut self, now: u64, up: Face, target: Face) {
        if self.roll.is_some_and(|r| now.saturating_sub(r.start) >= ROLL_MS) {
            self.roll = None;
        }
        if self.roll.is_none() {
            if let Some(about) = Self::step_toward(&self.turn, up, target) {
                let from = self.turn;
                self.turn = from.then(&Turn::quarter_about(about));
                self.roll = Some(Roll {
                    from,
                    about,
                    start: now,
                });
            }
        }
    }

    /// Put maze face `target` on the die's `up` face at once.
    pub fn snap(&mut self, up: Face, target: Face) {
        self.roll = None;
        while let Some(about) = Self::step_toward(&self.turn, up, target) {
            self.turn = self.turn.then(&Turn::quarter_about(about));
        }
    }

    /// The way through the maze that the die direction `d` stands for.
    pub fn maze_way(&self, d: Face) -> Face {
        face_along(self.turn.apply(axis(d)))
    }

    /// What the die's `face` shows at `now`: up to two maze faces, each with
    /// the transform that lays its canvas onto the screen. While the map
    /// rolls, faces round the roll slide one maze face off and the next on,
    /// and the two on its axis turn.
    pub fn layers(&self, now: u64, face: Face) -> Vec<(Face, Transform), 2> {
        let mut out = Vec::new();
        let settled = |t: &Turn| (t.maze_face(face), Transform::quarter(t.quarter(face)));
        let Some(roll) = self.roll else {
            let _ = out.push(settled(&self.turn));
            return out;
        };
        let u = crate::ui::ease((now.saturating_sub(roll.start)) as f32 / ROLL_MS as f32);
        let n = axis(face);
        if dot(n, roll.about) != 0 {
            let (c0, s0) = roll.from.cos_sin(face);
            let (c1, s1) = self.turn.cos_sin(face);
            let a0 = libm::atan2f(s0 as f32, c0 as f32);
            let mut da = libm::atan2f(s1 as f32, c1 as f32) - a0;
            if da > core::f32::consts::PI {
                da -= core::f32::consts::TAU;
            } else if da < -core::f32::consts::PI {
                da += core::f32::consts::TAU;
            }
            let _ = out.push((roll.from.maze_face(face), Transform::rotated(a0 + da * u)));
            return out;
        }
        // The content moves away from the face whose maze comes onto this
        // one (`about × n`).
        let f = &BASES[face.index()];
        let v = scale(cross(roll.about, n), -1);
        let (dx, dy) = (dot(v, ivec(f.x)) as f32, -dot(v, ivec(f.y)) as f32);
        let w = 2.0 * HALF;
        let (old, new) = (settled(&roll.from), settled(&self.turn));
        let _ = out.push((old.0, old.1.offset(dx * w * u, dy * w * u)));
        let back = -(1.0 - u) * w;
        let _ = out.push((new.0, new.1.offset(dx * back, dy * back)));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_cell() -> impl Iterator<Item = Cell> {
        (0..CELLS as u16).map(Cell)
    }

    #[test]
    fn cells_sit_on_their_faces() {
        for c in every_cell() {
            let p = c.pos();
            assert_eq!(dot(p, axis(c.face())), OUT);
            assert_eq!(Cell::at(c.face(), p), c);
        }
    }

    #[test]
    fn every_step_can_be_walked_back() {
        for c in every_cell() {
            for d in ways(c.face()) {
                let (next, arrive) = c.step(d);
                assert!(slot(next.face(), arrive).is_some(), "arrives along the face");
                assert_eq!(next.step(arrive.opposite()), (c, d.opposite()), "{c:?} {d:?}");
            }
        }
    }

    #[test]
    fn four_steps_over_an_edge_come_round_the_die() {
        // 32 steps straight on is once round the die.
        let mut w = (START, Face::PosX);
        for _ in 0..4 * N {
            w = w.0.step(w.1);
        }
        assert_eq!(w, (START, Face::PosX));
    }

    fn maze(seed: u32) -> Maze {
        let mut m = Maze::new();
        m.generate(&mut Rng::new(seed));
        m
    }

    #[test]
    fn the_maze_is_connected_with_no_dead_ends() {
        for seed in 1..20 {
            let m = maze(seed);
            let mut seen = [false; CELLS];
            let mut todo: Vec<Cell, CELLS> = Vec::new();
            seen[0] = true;
            let _ = todo.push(Cell(0));
            while let Some(c) = todo.pop() {
                assert!(m.exits(c) >= 2, "dead end at {c:?}");
                for d in ways(c.face()) {
                    let next = c.step(d).0;
                    if m.open(c, d) && !seen[next.index()] {
                        assert!(m.open(next, c.step(d).1.opposite()), "walls agree");
                        seen[next.index()] = true;
                        let _ = todo.push(next);
                    }
                }
            }
            assert!(seen.iter().all(|s| *s), "every cell reached");
        }
    }

    #[test]
    fn a_quarter_turn_is_a_rotation() {
        for f in Face::ALL {
            let q = Turn::quarter_about(axis(f));
            assert_eq!(q.apply(axis(f)), axis(f));
            let four = q.then(&q).then(&q).then(&q);
            assert_eq!(four, Turn::IDENTITY);
        }
    }

    #[test]
    fn the_view_keeps_the_runners_face_on_top() {
        for up in Face::ALL {
            for target in Face::ALL {
                let mut v = View::new();
                v.snap(up, target);
                assert_eq!(v.turn().maze_face(up), target);
                // Rolled one step at a time it gets there too, in at most two.
                let mut v = View::new();
                let mut now = 0;
                for _ in 0..3 {
                    v.update(now, up, target);
                    now += ROLL_MS;
                }
                assert_eq!(v.turn().maze_face(up), target, "{up:?} {target:?}");
            }
        }
    }

    #[test]
    fn neighbouring_screens_show_neighbouring_maze_faces() {
        // Whatever the turn, a die edge shows a maze edge, lined up: the
        // cell by the edge on one screen steps onto the cell by the edge on
        // the other.
        let mut v = View::new();
        v.snap(Face::PosZ, Face::NegX);
        let t = v.turn();
        for f in Face::ALL {
            let g = t.maze_face(f);
            for d in ways(f) {
                assert_eq!(t.maze_face(d), face_along(t.apply(axis(d))));
                let way = v.maze_way(d);
                assert!(slot(g, way).is_some());
                assert_eq!(t.maze_face(d), way, "the screen that way shows the maze that way");
            }
        }
    }

    #[test]
    fn layers_line_up_with_the_turn() {
        let mut v = View::new();
        v.snap(Face::PosY, Face::PosY);
        // A cell's canvas position, laid onto its screen, points the same way
        // in the die as the cell does.
        for f in Face::ALL {
            let layers = v.layers(0, f);
            let (g, xf) = layers[0];
            let c = Cell::new(g, 7, 3);
            let (x, y) = c.canvas();
            let (px, py) = xf.forward(x, y);
            let b = &BASES[f.index()];
            let die = v.turn().unapply(c.pos());
            let (ex, ey) = (dot(die, ivec(b.x)) as f32, -dot(die, ivec(b.y)) as f32);
            assert!((px - 48.0) * ex >= 0.0 && (py - 48.0) * ey >= 0.0, "{f:?}");
        }
    }

    fn game() -> (Game, u64) {
        let mut g = Game::new(3);
        g.start(0, 1234);
        (g, 0)
    }

    /// Step the game `secs` at 60 Hz, collecting its events.
    fn play(g: &mut Game, now: &mut u64, secs: f32, want: Option<Face>) -> std::vec::Vec<RunEvent> {
        let mut out = std::vec::Vec::new();
        for _ in 0..(secs * 60.0) as u32 {
            *now += 16;
            out.extend(g.step(*now, 1.0 / 60.0, want));
        }
        out
    }

    #[test]
    fn a_game_starts_with_crystals_lying_about() {
        let (g, _) = game();
        assert_eq!(g.phase(), Phase::Playing);
        assert_eq!(g.crystals().len(), CRYSTALS);
        assert!(!g.has_crystal(START));
        assert_eq!((g.score(), g.lives(), g.bursts()), (0, LIVES, BURSTS_START));
    }

    #[test]
    fn a_crystal_taken_turns_up_somewhere_else() {
        let (mut g, mut now) = game();
        // Put a crystal just ahead of the cube.
        let r = g.runner;
        let (next, _) = r.ahead();
        g.crystals[0] = next;
        let events = play(&mut g, &mut now, 0.5, None);
        assert!(events.contains(&RunEvent::Crystal));
        assert_eq!(g.score(), 1);
        assert_eq!(g.crystals().len(), CRYSTALS, "another one turned up");
        assert!(!g.has_crystal(next));
    }

    #[test]
    fn every_few_crystals_earn_a_burst() {
        let (mut g, _) = game();
        g.score = CRYSTALS_PER_BURST - 1;
        let (next, _) = g.runner.ahead();
        g.crystals[0] = next;
        let mut now = 0;
        let events = play(&mut g, &mut now, 0.5, None);
        assert!(events.contains(&RunEvent::Earned));
        assert_eq!(g.bursts(), BURSTS_START + 1);
    }

    #[test]
    fn the_runner_turns_when_asked_and_reverses_at_once() {
        let (mut g, _) = game();
        let back = g.runner.dir.opposite();
        g.step(16, 0.2, None);
        assert!(g.runner.moving);
        g.step(32, 0.0, Some(back));
        assert_eq!(g.runner.dir, back);
    }

    #[test]
    fn more_ants_hatch_as_the_score_climbs() {
        let (mut g, _) = game();
        let out = |g: &Game| g.ants().iter().filter(|a| a.state != AntState::Nest).count();
        g.hatch(1_000);
        assert_eq!(out(&g), 0);
        g.hatch(HATCH_FIRST_MS);
        assert_eq!(out(&g), 1, "the first a moment in");
        let t = HATCH_FIRST_MS + 10 * HATCH_GAP_MS;
        g.hatch(t);
        assert_eq!(out(&g), 1, "no more until there's sugar");
        g.score = 2 * CRYSTALS_PER_ANT;
        g.hatch(t);
        assert_eq!(out(&g), 2);
        g.hatch(t + 100);
        assert_eq!(out(&g), 2, "spaced out");
        g.hatch(t + HATCH_GAP_MS);
        assert_eq!(out(&g), 3);
        g.score = 100;
        g.hatch(t + 5 * HATCH_GAP_MS);
        assert_eq!(out(&g), 3, "never more than the Ants setting");
    }

    #[test]
    fn the_cube_leaves_a_scent_that_fades() {
        let (mut g, mut now) = game();
        let start = g.runner.cell;
        play(&mut g, &mut now, 0.5, None);
        assert_ne!(g.runner.cell, start);
        let fresh = g.scent(start, now);
        assert!(fresh > 0.8, "{fresh}");
        assert_eq!(g.scent(start, now + SCENT_MS), 0.0);
        assert_eq!(g.scent(Cell::new(Face::NegZ, 0, 0), now), 0.0, "never visited");
    }

    #[test]
    fn an_ant_on_the_trail_follows_it() {
        let (mut g, _) = game();
        let now = 5_000;
        let runner = g.runner.cell.pos();
        // A cell on the far side, well away from the cube, with ways out.
        let cell = (0..CELLS as u16)
            .map(Cell)
            .find(|c| c.face() == START.face().opposite() && g.maze.exits(*c) >= 3)
            .expect("a junction");
        let open: std::vec::Vec<Face> = ways(cell.face())
            .into_iter()
            .filter(|d| g.maze.open(cell, *d))
            .collect();
        // The cube passed here 3 s ago and went on along `ahead` a second
        // later: the trail is fresher that way.
        let ahead = open[1];
        g.scent[cell.index()] = now as u32 - 3_000 + 1;
        g.scent[cell.step(ahead).0.index()] = now as u32 - 2_000 + 1;
        for _ in 0..20 {
            assert_eq!(g.choose(cell, open[0], runner, now, true), ahead);
        }
        // With the trail gone cold, it wanders: sometimes another way.
        let cold = now + SCENT_MS + 1;
        let ways_taken: std::collections::BTreeSet<usize> = (0..40)
            .map(|_| g.choose(cell, open[0], runner, cold, true).index())
            .collect();
        assert!(ways_taken.len() > 1);
    }

    #[test]
    fn standing_still_the_cube_is_caught_and_the_game_ends() {
        let mut g = Game::new(4);
        g.start(0, 99);
        g.score = 20; // every ant out
        let mut now = 0;
        let mut caught = 0;
        let mut over = false;
        while now < 900_000 && !over {
            now += 16;
            for e in g.step(now, 1.0 / 60.0, None) {
                caught += (e == RunEvent::Caught) as u32;
                over |= e == RunEvent::GameOver;
            }
        }
        assert!(over, "the ants found it");
        assert_eq!(caught, LIVES as u32);
        assert!(matches!(g.phase(), Phase::Over { .. }));
        // A shake starts again.
        assert_eq!(g.start(now, 5).as_slice(), &[RunEvent::Started]);
        assert_eq!(g.lives(), LIVES);
    }

    #[test]
    fn a_burst_dazes_nearby_ants_and_they_cant_catch() {
        let (mut g, mut now) = game();
        play(&mut g, &mut now, 2.1, None);
        let k = g.ants().iter().position(|a| a.state == AntState::Out).unwrap();
        // The ant right on the cube: a burst and it's harmless.
        g.ants[k].w = Walker::at(g.runner.cell, g.runner.dir);
        assert_eq!(g.burst(now).as_slice(), &[RunEvent::Burst]);
        assert_eq!(g.bursts(), 0);
        assert!(matches!(g.ants[k].state, AntState::Dazed { .. }));
        let events = g.step(now + 16, 0.0, None);
        assert!(!events.contains(&RunEvent::Caught));
        // Out of bursts: a shake does nothing.
        assert!(g.burst(now + 32).is_empty());
        // Once the daze wears off it's back on its feet.
        let far = Cell::new(START.face().opposite(), 0, 0);
        g.ants[k].w = Walker::at(far, ways(far.face())[0]);
        g.step(now + DAZE_MS + 16, 0.0, None);
        assert_eq!(g.ants[k].state, AntState::Out);
    }

    #[test]
    fn a_pause_holds_everything() {
        let (mut g, _) = game();
        g.step(100, 0.1, None);
        let r = g.runner;
        g.pause();
        g.step(200, 0.1, None);
        assert_eq!(g.runner, r);
        g.resume(4_900);
        assert_eq!(g.phase(), Phase::Playing);
    }
}
