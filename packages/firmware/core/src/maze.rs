//! Sugar Run: a maze over the whole cube.
//!
//! Every face is an 8×8 grid of cells, and the corridors carry on over the
//! edges, so the maze has no border: it is the surface of the die. You chase
//! sugar round it and ants chase you.
//!
//! The maze is locked to the die, like a printed one: each screen always
//! shows the same part of it. When the runner runs off the top screen onto
//! a side, you tip the die to bring that screen up and follow it. Lean the
//! runner's screen to steer; the runner goes downhill, turning at the next
//! opening like a joystick held over. Each life starts on whichever screen
//! is up, with the ants' nest on the one underneath.
//!
//! ```text
//!   Ready ── shake ──▶ Playing ── caught, lives left ──▶ Caught ──▶ Playing
//!                       │  ▲  └── every dot eaten ──▶ Cleared ──▶ Playing (next level)
//!           set down ── │  └── tilt
//!                       ▼                       caught, no lives ──▶ Over ── shake ──▶ Playing
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

use crate::orientation::BASES;

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

/// Runner and ant speeds, cells a second, at level 1 and the most they
/// reach.
const RUNNER_SPEED: f32 = 4.2;
const RUNNER_MAX: f32 = 5.4;
const ANT_SPEED: f32 = 3.3;
const ANT_MAX: f32 = 5.0;
/// A scared ant's speed, against its normal one.
const SCARED_SLOW: f32 = 0.55;
/// How long a sugar lump scares the ants, at level 1, and the least it does.
const SCARE_MS: u64 = 6_500;
const SCARE_MIN_MS: u64 = 2_500;
/// The last part of a scare, when the ants flicker.
pub const SCARE_WARN_MS: u64 = 1_800;
/// Ants leave the nest one by one: the first this long after the start,
/// the rest this far apart.
const NEST_FIRST_MS: u64 = 1_500;
const NEST_GAP_MS: u64 = 2_500;
/// An eaten ant waits in the nest this long before it comes out again.
const EATEN_WAIT_MS: u64 = 3_000;
/// Ants wander toward their corners, then hunt, turn and turn about.
const WANDER_MS: u64 = 6_000;
const HUNT_MS: u64 = 20_000;
/// How close (doubled units) an ant must come to catch the runner.
const CATCH: f32 = 1.2;
/// How long the caught runner shrinks away, and a cleared maze flashes,
/// before play goes on.
pub const CAUGHT_MS: u64 = 1_800;
pub const CLEARED_MS: u64 = 2_600;

/// Points for a crystal, a sugar lump and the first ant eaten on one lump
/// (each more on the same lump doubles).
pub const DOT_POINTS: u32 = 10;
pub const LUMP_POINTS: u32 = 50;
pub const ANT_POINTS: u32 = 200;

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
    /// Waiting in the nest until then.
    Nest {
        until: u64,
    },
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ant {
    pub w: Walker,
    pub state: AntState,
    /// Running from the runner (it ate a sugar lump since this ant came
    /// out).
    pub scared: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Nothing started: the die shows its setup label.
    Ready,
    Playing,
    /// Set down: the maze waits, dimmed, until the die is tilted again.
    Paused,
    /// An ant got the runner.
    Caught {
        since: u64,
    },
    /// Every crystal eaten.
    Cleared {
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
    Dot,
    Lump,
    AteAnt,
    Caught,
    Cleared,
    GameOver,
}

pub type RunEvents = Vec<RunEvent, 8>;

/// Where the runner starts a life on the `top` face.
pub const fn start_cell(top: Face) -> Cell {
    Cell::new(top, 3, 4)
}

/// Ant `k`'s place in the nest, in the middle of the face under `top`.
const fn nest_cell(top: Face, k: usize) -> Cell {
    const AT: [(usize, usize); MAX_ANTS as usize] = [(3, 3), (4, 4), (4, 3), (3, 4)];
    Cell::new(top.opposite(), AT[k].0, AT[k].1)
}
/// The corner each ant heads for while it wanders.
const CORNERS: [V3; MAX_ANTS as usize] = [[9, 9, 9], [-9, -9, 9], [9, -9, -9], [-9, 9, -9]];
/// Where the sugar lumps are, one per face.
const LUMPS: [(usize, usize); FACE_COUNT] = [(0, 0), (7, 7), (0, 7), (7, 0), (7, 7), (0, 0)];

pub struct Game {
    maze: Maze,
    /// Crystals left, a bit per cell.
    dots: [u8; CELLS / 8],
    lumps: [u8; CELLS / 8],
    dots_left: u16,
    pub runner: Walker,
    /// The face that's up, as of the last step: a life starts there, the
    /// ants under it.
    top: Face,
    ants: [Ant; MAX_ANTS as usize],
    ant_count: u8,
    phase: Phase,
    score: u32,
    lives: u8,
    level: u8,
    /// When the round (this life) began: the ants' clock.
    round_start: u64,
    scared_until: u64,
    /// Ants eaten on the current lump.
    chain: u8,
    rng: Rng,
    /// Seconds the runner has been moving, for its chomp.
    chomp: f32,
}

impl Game {
    pub fn new(ants: u8) -> Self {
        Self {
            maze: Maze::new(),
            dots: [0; CELLS / 8],
            lumps: [0; CELLS / 8],
            dots_left: 0,
            runner: Walker::at(start_cell(Face::PosY), Face::PosX),
            top: Face::PosY,
            ants: [Ant {
                w: Walker::at(nest_cell(Face::PosY, 0), Face::PosX),
                state: AntState::Nest { until: 0 },
                scared: false,
            }; MAX_ANTS as usize],
            ant_count: ants.clamp(MIN_ANTS, MAX_ANTS),
            phase: Phase::Ready,
            score: 0,
            lives: LIVES,
            level: 1,
            round_start: 0,
            scared_until: 0,
            chain: 0,
            rng: Rng::new(1),
            chomp: 0.0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn maze(&self) -> &Maze {
        &self.maze
    }

    pub fn score(&self) -> u32 {
        self.score
    }

    pub fn lives(&self) -> u8 {
        self.lives
    }

    pub fn level(&self) -> u8 {
        self.level
    }

    pub fn dots_left(&self) -> u16 {
        self.dots_left
    }

    /// How many ants give chase.
    pub fn ant_count(&self) -> u8 {
        self.ant_count
    }

    pub fn ants(&self) -> &[Ant] {
        &self.ants[..self.ant_count as usize]
    }

    pub fn chomp(&self) -> f32 {
        self.chomp
    }

    /// A game is under way (not waiting to start, and not over).
    pub fn in_play(&self) -> bool {
        !matches!(self.phase, Phase::Ready | Phase::Over { .. })
    }

    /// The maze is moving: nothing should interrupt it.
    pub fn running(&self) -> bool {
        matches!(
            self.phase,
            Phase::Playing | Phase::Caught { .. } | Phase::Cleared { .. }
        )
    }

    pub fn has_dot(&self, c: Cell) -> bool {
        bit(&self.dots, c)
    }

    pub fn has_lump(&self, c: Cell) -> bool {
        bit(&self.lumps, c)
    }

    /// How scared the ants are: 1 while fresh, flickering toward 0 at the
    /// end, 0 once it's over.
    pub fn scared_left_ms(&self, now: u64) -> u64 {
        self.scared_until.saturating_sub(now)
    }

    /// Start a game, or a new one once it's over (the firmware does this
    /// when a shake settles), the runner on the `top` face.
    pub fn start(&mut self, now: u64, seed: u32, top: Face) -> RunEvents {
        let mut out = RunEvents::new();
        if matches!(self.phase, Phase::Ready | Phase::Over { .. }) {
            self.rng = Rng::new(seed);
            self.score = 0;
            self.lives = LIVES;
            self.level = 1;
            self.top = top;
            self.new_maze();
            self.new_round(now);
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

    /// Picked up and tilted: carry on. The ants' timers don't count the
    /// pause.
    pub fn resume(&mut self, paused_ms: u64) {
        if self.phase == Phase::Paused {
            self.phase = Phase::Playing;
            self.round_start += paused_ms;
            if self.scared_until > 0 {
                self.scared_until += paused_ms;
            }
            for a in &mut self.ants {
                if let AntState::Nest { until } = &mut a.state {
                    *until += paused_ms;
                }
            }
        }
    }

    /// Back to the start, as when the mode is left.
    pub fn reset(&mut self) {
        *self = Self::new(self.ant_count);
    }

    fn new_maze(&mut self) {
        self.maze.generate(&mut self.rng);
        self.dots = [0xff; CELLS / 8];
        self.lumps = [0; CELLS / 8];
        for (face, (i, j)) in Face::ALL.into_iter().zip(LUMPS) {
            let c = Cell::new(face, i, j);
            clear_bit(&mut self.dots, c);
            set_bit(&mut self.lumps, c);
        }
        self.dots_left = self
            .dots
            .iter()
            .chain(&self.lumps)
            .map(|b| b.count_ones() as u16)
            .sum();
    }

    /// Everyone back to the start for a new life or level: the runner on
    /// the face that's up, the ants in the nest under it.
    fn new_round(&mut self, now: u64) {
        let start = start_cell(self.top);
        let start_dir = ways(self.top)
            .into_iter()
            .find(|d| self.maze.open(start, *d))
            .unwrap_or(Face::PosX);
        self.runner = Walker::at(start, start_dir);
        if take_bit(&mut self.dots, start) {
            self.dots_left -= 1;
        }
        for (k, a) in self.ants.iter_mut().enumerate() {
            let cell = nest_cell(self.top, k);
            a.w = Walker::at(cell, ways(cell.face())[k % 4]);
            a.state = AntState::Nest {
                until: now + NEST_FIRST_MS + NEST_GAP_MS * k as u64,
            };
            a.scared = false;
        }
        self.round_start = now;
        self.scared_until = 0;
        self.chain = 0;
        self.phase = Phase::Playing;
    }

    fn runner_speed(&self) -> f32 {
        (RUNNER_SPEED + 0.2 * (self.level - 1) as f32).min(RUNNER_MAX)
    }

    fn ant_speed(&self) -> f32 {
        (ANT_SPEED + 0.25 * (self.level - 1) as f32).min(ANT_MAX)
    }

    fn scare_ms(&self) -> u64 {
        SCARE_MS
            .saturating_sub(700 * (self.level - 1) as u64)
            .max(SCARE_MIN_MS)
    }

    /// Advance by `dt` seconds. `want` is the way the runner's screen is
    /// leaning (a way along it, or it's ignored), and `top` the face that's
    /// up, where the next life starts.
    pub fn step(&mut self, now: u64, dt: f32, want: Option<Face>, top: Face) -> RunEvents {
        let mut out = RunEvents::new();
        self.top = top;
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
            Phase::Cleared { since } if now.saturating_sub(since) >= CLEARED_MS => {
                self.level = self.level.saturating_add(1);
                self.new_maze();
                self.new_round(now);
                return out;
            }
            _ => return out,
        }
        let dt = dt.clamp(0.0, 0.1);
        self.move_runner(dt, want, &mut out);
        if self.dots_left == 0 {
            self.phase = Phase::Cleared { since: now };
            let _ = out.push(RunEvent::Cleared);
            return out;
        }
        if out.contains(&RunEvent::Lump) {
            self.scared_until = now + self.scare_ms();
            self.chain = 0;
            for a in &mut self.ants {
                if a.state == AntState::Out {
                    a.scared = true;
                    a.w.reverse();
                }
            }
        }
        if now >= self.scared_until {
            for a in &mut self.ants {
                a.scared = false;
            }
        }
        for k in 0..self.ant_count as usize {
            self.move_ant(k, now, dt);
        }
        self.meet(now, &mut out);
        out
    }

    fn move_runner(&mut self, dt: f32, want: Option<Face>, out: &mut RunEvents) {
        let speed = self.runner_speed();
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
        self.chomp += dt;
        r.t += speed * dt;
        while r.t >= 1.0 {
            let (next, arrive) = r.ahead();
            r.cell = next;
            r.dir = arrive;
            r.t -= 1.0;
            if take_bit(&mut self.dots, next) {
                self.dots_left -= 1;
                self.score += DOT_POINTS;
                let _ = out.push(RunEvent::Dot);
            }
            if take_bit(&mut self.lumps, next) {
                self.dots_left -= 1;
                self.score += LUMP_POINTS;
                let _ = out.push(RunEvent::Lump);
            }
            if let Some(w) = want.filter(|w| self.maze.open(next, *w)) {
                r.dir = w;
            } else if !self.maze.open(next, r.dir) {
                r.t = 0.0;
                r.moving = false;
            }
        }
    }

    fn move_ant(&mut self, k: usize, now: u64, dt: f32) {
        let hunting = (now.saturating_sub(self.round_start)) % (WANDER_MS + HUNT_MS) >= WANDER_MS;
        let target = self.target(k, hunting);
        let speed = self.ant_speed() * if self.ants[k].scared { SCARED_SLOW } else { 1.0 };
        let a = &mut self.ants[k];
        match a.state {
            AntState::Nest { until } if now >= until => {
                a.state = AntState::Out;
                a.w.moving = true;
            }
            AntState::Nest { .. } => return,
            AntState::Out => {}
        }
        a.w.t += speed * dt;
        // Out of the nest the first time, or blocked: pick a way now.
        if !self.maze.open(a.w.cell, a.w.dir) && a.w.t < 1.0 {
            a.w.t = 0.0;
            a.w.dir = choose(
                &self.maze,
                &mut self.rng,
                a.w.cell,
                a.w.dir,
                target,
                a.scared,
                true,
            );
            return;
        }
        while a.w.t >= 1.0 {
            let (next, arrive) = a.w.ahead();
            a.w.cell = next;
            a.w.t -= 1.0;
            a.w.dir = choose(&self.maze, &mut self.rng, next, arrive, target, a.scared, false);
        }
    }

    /// Where ant `k` heads: its corner while wandering; while hunting the
    /// first goes for the runner, the second for where it's heading, the
    /// third for where it's heading from, and the fourth hunts only from
    /// afar and wanders once it's close.
    fn target(&self, k: usize, hunting: bool) -> V3 {
        let runner = self.runner.cell.pos();
        if !hunting {
            return CORNERS[k];
        }
        let ahead = add(runner, scale(axis(self.runner.dir), 8));
        match k {
            0 => runner,
            1 => ahead,
            2 => add(runner, scale(axis(self.runner.dir), -6)),
            _ => {
                let d = add(self.ants[k].w.cell.pos(), scale(runner, -1));
                if dot(d, d) > 12 * 12 {
                    runner
                } else {
                    CORNERS[k]
                }
            }
        }
    }

    /// Did the runner and an ant meet? A scared one is eaten; any other
    /// catches the runner.
    fn meet(&mut self, now: u64, out: &mut RunEvents) {
        let p = self.runner.pos();
        let n = self.ant_count as usize;
        let top = self.top;
        for (k, a) in self.ants[..n].iter_mut().enumerate() {
            let home = nest_cell(top, k);
            if a.state != AntState::Out {
                continue;
            }
            let q = a.w.pos();
            let d2: f32 = (0..3).map(|i| (p[i] - q[i]) * (p[i] - q[i])).sum();
            if d2 >= CATCH * CATCH {
                continue;
            }
            if a.scared {
                self.score += ANT_POINTS << self.chain.min(3);
                self.chain += 1;
                a.w = Walker::at(home, ways(home.face())[k % 4]);
                a.state = AntState::Nest {
                    until: now + EATEN_WAIT_MS,
                };
                a.scared = false;
                let _ = out.push(RunEvent::AteAnt);
            } else {
                self.lives = self.lives.saturating_sub(1);
                self.runner.moving = false;
                self.phase = Phase::Caught { since: now };
                let _ = out.push(RunEvent::Caught);
                return;
            }
        }
    }
}

/// An ant at a cell centre, arriving along `arrive`, picks its way: never
/// straight back unless it must, and the way toward `target` (at random
/// while scared). `fresh` lets it turn back (it has just come out).
fn choose(
    maze: &Maze,
    rng: &mut Rng,
    cell: Cell,
    arrive: Face,
    target: V3,
    scared: bool,
    fresh: bool,
) -> Face {
    let mut options: Vec<Face, 4> = Vec::new();
    for d in ways(cell.face()) {
        if maze.open(cell, d) && (fresh || d != arrive.opposite()) {
            let _ = options.push(d);
        }
    }
    if options.is_empty() {
        return arrive.opposite();
    }
    if scared {
        return options[rng.below(options.len() as u32) as usize];
    }
    // Nearest by direction from the die's centre, not in a straight line:
    // straight through the die, the middle of the far face is nearest to
    // everything on this one, and an ant there would never leave it.
    let near = |d: &Face| {
        let p = cell.step(*d).0.pos();
        dot(p, target) as f32 / libm::sqrtf(dot(p, p) as f32)
    };
    options
        .iter()
        .copied()
        .max_by(|a, b| near(a).total_cmp(&near(b)))
        .unwrap_or(arrive)
}

fn bit(bits: &[u8; CELLS / 8], c: Cell) -> bool {
    bits[c.index() / 8] & (1 << (c.index() % 8)) != 0
}

fn set_bit(bits: &mut [u8; CELLS / 8], c: Cell) {
    bits[c.index() / 8] |= 1 << (c.index() % 8);
}

fn clear_bit(bits: &mut [u8; CELLS / 8], c: Cell) {
    bits[c.index() / 8] &= !(1 << (c.index() % 8));
}

/// Clears the bit and says whether it was set.
fn take_bit(bits: &mut [u8; CELLS / 8], c: Cell) -> bool {
    let had = bit(bits, c);
    clear_bit(bits, c);
    had
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
        let start = start_cell(Face::PosY);
        let mut w = (start, Face::PosX);
        for _ in 0..4 * N {
            w = w.0.step(w.1);
        }
        assert_eq!(w, (start, Face::PosX));
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

    fn game() -> (Game, u64) {
        let mut g = Game::new(3);
        g.start(0, 1234, Face::PosY);
        (g, 0)
    }

    #[test]
    fn a_shake_starts_and_the_runner_eats() {
        let (mut g, mut now) = game();
        assert_eq!(g.phase(), Phase::Playing);
        let before = g.dots_left();
        let mut dots = 0;
        for _ in 0..60 {
            now += 16;
            dots += g
                .step(now, 1.0 / 60.0, None, Face::PosY)
                .iter()
                .filter(|e| **e == RunEvent::Dot)
                .count();
        }
        assert!(dots > 0, "it ran into crystals");
        assert_eq!(before - g.dots_left(), dots as u16);
        assert_eq!(g.score(), DOT_POINTS * dots as u32);
    }

    #[test]
    fn the_runner_turns_when_asked_and_reverses_at_once() {
        let (mut g, _) = game();
        let back = g.runner.dir.opposite();
        g.step(16, 0.2, None, Face::PosY);
        assert!(g.runner.moving);
        g.step(32, 0.0, Some(back), Face::PosY);
        assert_eq!(g.runner.dir, back);
    }

    #[test]
    fn ants_leave_the_nest_one_by_one() {
        let (mut g, mut now) = game();
        let out = |g: &Game| g.ants().iter().filter(|a| a.state == AntState::Out).count();
        while now < NEST_FIRST_MS + 100 {
            now += 16;
            g.step(now, 1.0 / 60.0, None, Face::PosY);
        }
        assert_eq!(out(&g), 1);
        while now < NEST_FIRST_MS + 2 * NEST_GAP_MS + 100 && g.phase() == Phase::Playing {
            now += 16;
            g.step(now, 1.0 / 60.0, None, Face::PosY);
        }
        assert!(g.phase() != Phase::Playing || out(&g) == 3);
    }

    #[test]
    fn standing_still_the_runner_is_caught_and_the_game_ends() {
        let mut g = Game::new(4);
        g.start(0, 99, Face::PosY);
        let mut now = 0;
        let mut caught = 0;
        let mut over = false;
        // Walk into the first wall and stay there.
        while now < 600_000 && !over {
            now += 16;
            for e in g.step(now, 1.0 / 60.0, None, Face::PosY) {
                caught += (e == RunEvent::Caught) as u32;
                over |= e == RunEvent::GameOver;
            }
        }
        assert!(over, "the ants found it");
        assert_eq!(caught, LIVES as u32);
        assert!(matches!(g.phase(), Phase::Over { .. }));
        // A shake starts again.
        assert_eq!(g.start(now, 5, Face::PosY).as_slice(), &[RunEvent::Started]);
        assert_eq!(g.lives(), LIVES);
    }

    #[test]
    fn each_life_starts_on_the_face_that_is_up() {
        let mut g = Game::new(2);
        g.start(0, 3, Face::PosZ);
        assert_eq!(g.runner.cell.face(), Face::PosZ);
        assert!(
            g.ants().iter().all(|a| a.w.cell.face() == Face::NegZ),
            "nest underneath"
        );
        assert!(!g.has_dot(g.runner.cell), "the start's crystal is taken");
        // Caught, with the die turned since: the next life is on the new top.
        g.phase = Phase::Caught { since: 0 };
        g.step(CAUGHT_MS, 0.0, None, Face::NegX);
        assert_eq!(g.phase(), Phase::Playing);
        assert_eq!(g.runner.cell.face(), Face::NegX);
        assert!(g.ants().iter().all(|a| a.w.cell.face() == Face::PosX));
    }

    #[test]
    fn a_lump_scares_the_ants_and_they_can_be_eaten() {
        let (mut g, mut now) = game();
        while g.ants().iter().all(|a| a.state != AntState::Out) {
            now += 16;
            g.step(now, 1.0 / 60.0, None, Face::PosY);
        }
        // Put the runner by a lump, then an ant right behind it.
        let lump = Cell::new(Face::PosY, LUMPS[2].0, LUMPS[2].1);
        let way = ways(lump.face())
            .into_iter()
            .find(|d| g.maze.open(lump, *d))
            .unwrap();
        let (from, arrive) = lump.step(way);
        g.runner = Walker {
            cell: from,
            dir: arrive.opposite(),
            t: 0.9,
            moving: true,
        };
        let out = g.step(now + 16, 0.05, None, Face::PosY);
        assert!(out.contains(&RunEvent::Lump));
        let k = g.ants().iter().position(|a| a.state == AntState::Out).unwrap();
        assert!(g.ants[k].scared);
        g.ants[k].w = Walker::at(g.runner.cell, g.runner.dir);
        let out = g.step(now + 32, 0.0, None, Face::PosY);
        assert!(out.contains(&RunEvent::AteAnt));
        assert_eq!(g.phase(), Phase::Playing);
        assert!(matches!(g.ants[k].state, AntState::Nest { .. }));
    }

    #[test]
    fn a_pause_holds_everything() {
        let (mut g, _) = game();
        g.step(100, 0.1, None, Face::PosY);
        let r = g.runner;
        g.pause();
        g.step(200, 0.1, None, Face::PosY);
        assert_eq!(g.runner, r);
        g.resume(4_900);
        assert_eq!(g.phase(), Phase::Playing);
        assert!(matches!(g.ants[0].state, AntState::Nest { until } if until > 5_000));
    }
}
