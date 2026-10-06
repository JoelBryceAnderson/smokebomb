//! Sugar Rush: a sliding-stick puzzle on the die's six screens.
//!
//! Every face shows a grid of sticks. Each stick has an arrow at its head
//! and only moves the way that arrow points, sliding off the edge of its
//! screen. A stick in the way stops it: a jam. Clear every stick to win the
//! level. Sticks can bend over the die's edges, so the puzzle is read by
//! turning the die.
//!
//! The die can't tell where on a screen a finger is, only which screens are
//! touched, and the IMU tells it which face is up and which way it leans.
//! So play is *tip and tap* (SIM_SPEC X3):
//!
//! * only the top screen is in play;
//! * leaning the die toward an edge picks that direction: the top screen's
//!   sticks that point downhill light, nearest their edge first, and one
//!   blinks. The direction stays picked when the die settles back level,
//!   so the tap needn't come while it leans, until the die is turned to
//!   another screen or leaned another way;
//! * a tap on the top screen slides the blinking stick off; a tap on any
//!   other screen blinks the next one;
//! * turning the die brings another screen to the top.
//!
//! Boards are built in reverse, so every one can be cleared: each new
//! stick's way out is clear of every stick placed before it, and those
//! leave after it.
//!
//! Geometry is in die coordinates at twice the cell scale, so every cell
//! centre is an integer: the cube spans `[-n, n]` on each axis for an `n`×`n`
//! board, and a cell centre sits on a face (one coordinate is ±n) with odd
//! coordinates on the other two axes.

use libm::{floorf, roundf, sinf, sqrtf};
use smokebomb_hal::{Color, Face, Target, FACE_COUNT};

use crate::gfx::{Painter, Style};
use crate::orientation::BASES;
use crate::palette64 as pal;

/// The biggest board: 4×4 cells a face.
pub const MAX_N: u8 = 4;
/// Cells in the biggest board.
pub const MAX_CELLS: usize = FACE_COUNT * (MAX_N as usize) * (MAX_N as usize);
/// The longest stick, in cells.
pub const MAX_LEN: usize = 6;
/// Every stick is at least two cells long.
pub const MIN_LEN: usize = 2;
/// The most sticks a board can hold.
pub const MAX_STICKS: usize = MAX_CELLS / MIN_LEN;
/// Jams allowed per level; the last one resets the board.
pub const JAMS: u8 = 3;

/// The level number on every screen before play starts.
pub const INTRO_MS: u64 = 1_400;
/// "Clear!" before the next level.
pub const CLEARED_MS: u64 = 2_600;
/// "Jammed" before the same board starts again.
pub const JAMMED_MS: u64 = 1_800;
/// A stick that hits something runs at it and back in this long.
pub const BUMP_MS: u64 = 340;
/// How long the stick that something hit flashes red.
pub const FLASH_MS: u64 = 700;

/// An axis vector or a cell centre, in doubled die coordinates.
pub type V = [i8; 3];
type F = [f32; 3];

fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn neg(a: V) -> V {
    [-a[0], -a[1], -a[2]]
}
fn dot(a: V, b: V) -> i32 {
    a.iter().zip(b).map(|(x, y)| *x as i32 * y as i32).sum()
}
fn axis_of(d: V) -> usize {
    d.iter().position(|c| *c != 0).unwrap_or(0)
}
fn fv(a: V) -> F {
    a.map(|c| c as f32)
}
fn fdot(a: F, b: F) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn fmix(a: F, b: F, t: f32) -> F {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}
fn fdist(a: F, b: F) -> f32 {
    sqrtf((0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum())
}

/// A face's screen axes (right, up) and outward normal, as integers.
pub fn basis(face: Face) -> (V, V, V) {
    let b = &BASES[face.index()];
    let r = |v: [f32; 3]| v.map(|c| roundf(c) as i8);
    (r(b.x), r(b.y), r(b.n))
}

/// The face whose outward normal is `n`.
pub fn face_of_normal(n: V) -> Face {
    Face::ALL
        .into_iter()
        .find(|f| basis(*f).2 == n)
        .unwrap_or(Face::PosY)
}

/// The face a cell centre lies on, for an `n`×`n` board.
pub fn face_of(p: V, n: u8) -> Face {
    let a = (0..3).find(|a| p[*a].unsigned_abs() == n).unwrap_or(1);
    let mut normal = [0; 3];
    normal[a] = p[a].signum();
    face_of_normal(normal)
}

/// A small xorshift generator: boards come from a seed, so the same seed
/// always builds the same board.
#[derive(Clone, Copy, Debug)]
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        // Spread the seed's bits, and never start at zero (xorshift's one
        // stuck state).
        let x = (seed ^ 0x85EB_CA6B).wrapping_mul(0x9E37_79B9);
        Self(if x == 0 { 1 } else { x })
    }
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u32) as usize
    }
    fn chance(&mut self, pct: u32) -> bool {
        self.next() % 100 < pct
    }
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i + 1);
            items.swap(i, j);
        }
    }
}

/// A move under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    /// Sliding off the edge of its screen, since `at`.
    Exit { at: u64 },
    /// Ran into the stick `steps` cells ahead and is coming back, since `at`.
    Bump { at: u64, steps: u8 },
}

/// One stick: its cells from tail to head, and the way its arrow points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stick {
    cells: [V; MAX_LEN],
    len: u8,
    /// The head's direction, along its screen.
    pub dir: V,
    /// Off the board (it may still be sliding away: see `moving`).
    pub out: bool,
    pub moving: Option<Move>,
}

impl Default for Stick {
    fn default() -> Self {
        Self {
            cells: [[0; 3]; MAX_LEN],
            len: 0,
            dir: [0; 3],
            out: true,
            moving: None,
        }
    }
}

impl Stick {
    /// Cells from tail to head.
    pub fn cells(&self) -> &[V] {
        &self.cells[..self.len as usize]
    }

    pub fn head(&self) -> V {
        self.cells[self.len as usize - 1]
    }

    /// Gone for good: off the board and done sliding.
    pub fn gone(&self) -> bool {
        self.out && self.moving.is_none()
    }
}

/// A board of sticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Board {
    /// Cells across a face.
    pub n: u8,
    sticks: [Stick; MAX_STICKS],
    count: u8,
    /// Which stick holds each cell (index + 1), 0 for none. Sticks that are
    /// out are left off.
    occ: [u8; MAX_CELLS],
}

impl Board {
    fn empty(n: u8) -> Self {
        Self {
            n: n.clamp(2, MAX_N),
            sticks: [Stick::default(); MAX_STICKS],
            count: 0,
            occ: [0; MAX_CELLS],
        }
    }

    /// Build a board: `n`×`n` cells a face, sticks up to `max_len` long,
    /// filling about `fill_pct` of the cells.
    pub fn generate(n: u8, seed: u32, max_len: usize, fill_pct: u32) -> Self {
        let mut b = Self::empty(n);
        let n = b.n;
        let max_len = max_len.clamp(MIN_LEN, MAX_LEN);
        let mut rng = Rng::new(seed);
        let mut heads: heapless::Vec<V, MAX_CELLS> = heapless::Vec::new();
        for face in Face::ALL {
            let (x, y, nn) = basis(face);
            for i in 0..n as i8 {
                for j in 0..n as i8 {
                    let u = 2 * i - n as i8 + 1;
                    let v = 2 * j - n as i8 + 1;
                    let p = [0, 1, 2].map(|a| nn[a] * n as i8 + x[a] * u + y[a] * v);
                    let _ = heads.push(p);
                }
            }
        }
        let total = heads.len();
        let target = total * fill_pct as usize / 100;
        let mut filled = 0;
        for _pass in 0..3 {
            if filled >= target {
                break;
            }
            rng.shuffle(&mut heads);
            for &head in heads.iter() {
                if filled >= target || b.count as usize >= MAX_STICKS {
                    break;
                }
                if b.at(head).is_some() {
                    continue;
                }
                let (x, y, _) = basis(face_of(head, n));
                let mut dirs = [x, neg(x), y, neg(y)];
                rng.shuffle(&mut dirs);
                for d in dirs {
                    if let Some(len) = b.try_place(head, d, max_len, &mut rng) {
                        filled += len;
                        break;
                    }
                }
            }
        }
        b
    }

    /// Try a stick with its head at `head` pointing `d`: its way out must be
    /// clear, and its body grows back from the head. Returns its length.
    fn try_place(&mut self, head: V, d: V, max_len: usize, rng: &mut Rng) -> Option<usize> {
        let lane = self.lane(head, d);
        if lane.iter().any(|c| self.at(*c).is_some()) {
            return None;
        }
        let want = MIN_LEN + rng.below(max_len - MIN_LEN + 1);
        let mut body: heapless::Vec<V, MAX_LEN> = heapless::Vec::new();
        let _ = body.push(head);
        let (mut cur, mut going) = (head, neg(d));
        while body.len() < want {
            let normal = basis(face_of(cur, self.n)).2;
            let side = cross(normal, going);
            let mut opts = [going, side, neg(side)];
            if body.len() == 1 {
                // Straight behind the head, so the arrow reads as one.
                opts = [going, going, going];
            } else if !rng.chance(55) {
                opts = [side, neg(side), going];
                if rng.chance(50) {
                    opts.swap(0, 1);
                }
            } else if rng.chance(50) {
                opts.swap(1, 2);
            }
            let next = opts
                .into_iter()
                .map(|o| self.step(cur, o))
                .find(|(c, _)| self.at(*c).is_none() && !lane.contains(c) && !body.contains(c));
            let Some((c, g)) = next else { break };
            let _ = body.push(c);
            cur = c;
            going = g;
        }
        if body.len() < MIN_LEN {
            return None;
        }
        let k = self.count as usize;
        let s = &mut self.sticks[k];
        s.len = body.len() as u8;
        for (i, c) in body.iter().rev().enumerate() {
            s.cells[i] = *c;
        }
        s.dir = d;
        s.out = false;
        s.moving = None;
        self.count += 1;
        for c in body.iter() {
            let i = self.index(*c);
            self.occ[i] = k as u8 + 1;
        }
        Some(body.len())
    }

    /// One cell on from `c` going `d`. Over an edge it lands on the next
    /// face, heading inward. Returns the cell and the direction it now goes.
    pub fn step(&self, c: V, d: V) -> (V, V) {
        let n = self.n as i8;
        let q = add(add(c, d), d);
        if q[axis_of(d)].abs() < n {
            return (q, d);
        }
        let normal = basis(face_of(c, self.n)).2;
        (add(add(c, d), neg(normal)), neg(normal))
    }

    /// The cells between `head` and the edge of its screen, going `d`.
    pub fn lane(&self, head: V, d: V) -> heapless::Vec<V, { MAX_N as usize }> {
        let mut out = heapless::Vec::new();
        let mut p = head;
        loop {
            let q = add(add(p, d), d);
            if q[axis_of(d)].abs() >= self.n as i8 {
                return out;
            }
            let _ = out.push(q);
            p = q;
        }
    }

    fn index(&self, p: V) -> usize {
        let face = face_of(p, self.n);
        let (x, y, _) = basis(face);
        let n = self.n as i32;
        let i = (dot(p, x) + n - 1) / 2;
        let j = (dot(p, y) + n - 1) / 2;
        face.index() * (MAX_N as usize * MAX_N as usize) + (i as usize) * MAX_N as usize + j as usize
    }

    /// The stick on a cell, if one is (sticks that are out don't count).
    pub fn at(&self, p: V) -> Option<usize> {
        match self.occ[self.index(p)] {
            0 => None,
            k => Some(k as usize - 1),
        }
    }

    pub fn sticks(&self) -> &[Stick] {
        &self.sticks[..self.count as usize]
    }

    /// Sticks still on the board.
    pub fn left(&self) -> usize {
        self.sticks().iter().filter(|s| !s.out).count()
    }

    /// The stick `k`'s face: where its head is.
    pub fn head_face(&self, k: usize) -> Face {
        face_of(self.sticks[k].head(), self.n)
    }

    /// The first stick in `k`'s way and how many cells ahead, or `None` if
    /// the way is clear.
    pub fn blocked(&self, k: usize) -> Option<(usize, u8)> {
        let s = &self.sticks[k];
        self.lane(s.head(), s.dir)
            .iter()
            .enumerate()
            .find_map(|(i, c)| self.at(*c).filter(|o| *o != k).map(|o| (o, i as u8)))
    }

    fn take_off(&mut self, k: usize) {
        self.sticks[k].out = true;
        for i in 0..self.sticks[k].len as usize {
            let cell = self.index(self.sticks[k].cells[i]);
            self.occ[cell] = 0;
        }
    }
}

fn cross(a: V, b: V) -> V {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Where the game is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A new level, not on the screens yet (the die may be booting, or in
    /// another mode): its number shows once it is.
    Ready,
    /// The level number, since `at`.
    Intro {
        at: u64,
    },
    Play,
    /// Every stick is off, since `at`.
    Cleared {
        at: u64,
    },
    /// The last jam, since `at`: the board starts again.
    Jammed {
        at: u64,
    },
}

/// What a tap did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    /// Nothing to do: no lean, or nothing on top points downhill.
    Nothing,
    /// The blinking stick moved on to the next one.
    Picked,
    /// The stick slid off.
    Slid,
    /// It ran into a stick; `last` if that was the last jam allowed.
    Jammed { last: bool },
    /// Skipped the level number or the "Clear!" screen.
    Skipped,
}

/// What [`Rush::tick`] noticed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    None,
    /// The last stick slid off.
    Cleared,
    /// "Clear!" is done: start the next level (call [`Rush::next_level`]).
    NextLevel,
    /// The jammed board started again.
    Restarted,
}

/// A game of Sugar Rush: the board, the level, and how the die is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rush {
    pub board: Board,
    pub level: u16,
    seed: u32,
    pub jams: u8,
    pub phase: Phase,
    /// The top face and the direction picked along it.
    aim: Option<(Face, V)>,
    /// The way the die leans along its top face right now, if it does.
    lean: Option<V>,
    /// Which of the lit sticks blinks.
    pick: u8,
    /// A stick hit by a jam, flashing until then.
    flash: Option<(u8, u64)>,
}

impl Rush {
    /// A new game at `level` on an `n`×`n` board.
    pub fn new(n: u8, level: u16, seed: u32) -> Self {
        let level = level.max(1);
        Self {
            board: Self::build(n, level, seed),
            level,
            seed,
            jams: 0,
            phase: Phase::Ready,
            aim: None,
            lean: None,
            pick: 0,
            flash: None,
        }
    }

    /// Longer sticks and fuller boards as the levels go up.
    fn build(n: u8, level: u16, seed: u32) -> Board {
        let max_len = (3 + level as usize / 3).min(MAX_LEN);
        let fill = if level <= 1 { 70 } else { 85 };
        Board::generate(n, seed.wrapping_add(level as u32 * 0x9E37), max_len, fill)
    }

    /// The board size this game is on.
    pub fn n(&self) -> u8 {
        self.board.n
    }

    /// The next level, on a board from `seed`.
    pub fn next_level(&mut self, seed: u32) {
        *self = Self::new(self.board.n, self.level.saturating_add(1), seed);
    }

    /// Where the die is: which face is up and which way it leans along that
    /// face (`None` when level). A lean picks its direction, and the pick
    /// stays when the die comes back level; leaning another way picks
    /// again, and turning another face up drops it. Returns true when a
    /// new pick lights something.
    pub fn aim(&mut self, top: Face, lean: Option<V>) -> bool {
        self.lean = lean;
        let kept = self.aim.filter(|(f, _)| *f == top);
        let aim = lean.map(|d| (top, d)).or(kept);
        if aim == self.aim {
            return false;
        }
        self.aim = aim;
        self.pick = 0;
        !self.lit().is_empty()
    }

    /// The direction picked, if one is.
    pub fn downhill(&self) -> Option<V> {
        self.aim.map(|(_, d)| d)
    }

    /// The way the die leans right now, if it does.
    pub fn lean(&self) -> Option<V> {
        self.lean
    }

    /// The top screen's sticks that point downhill, nearest their edge
    /// first. Empty when the die is level.
    pub fn lit(&self) -> heapless::Vec<u8, MAX_STICKS> {
        let mut out: heapless::Vec<u8, MAX_STICKS> = heapless::Vec::new();
        let Some((top, d)) = self.aim else {
            return out;
        };
        if self.phase != Phase::Play {
            return out;
        }
        for (k, s) in self.board.sticks().iter().enumerate() {
            if !s.out && s.moving.is_none() && s.dir == d && self.board.head_face(k) == top {
                let _ = out.push(k as u8);
            }
        }
        out.sort_unstable_by_key(|k| self.board.lane(self.board.sticks[*k as usize].head(), d).len());
        out
    }

    /// The stick that blinks: the one a tap on the top screen slides.
    pub fn selected(&self) -> Option<u8> {
        let lit = self.lit();
        (!lit.is_empty()).then(|| lit[self.pick as usize % lit.len()])
    }

    /// A tap on another screen: blink the next lit stick.
    pub fn next_pick(&mut self) -> Tap {
        if self.lit().len() > 1 {
            self.pick = self.pick.wrapping_add(1);
            Tap::Picked
        } else {
            Tap::Nothing
        }
    }

    /// A tap on the top screen: slide `stick` (the one that was blinking
    /// when the finger landed). In the level number or "Clear!", skip on.
    pub fn slide(&mut self, stick: Option<u8>, now: u64) -> Tap {
        match self.phase {
            Phase::Ready | Phase::Intro { .. } => {
                self.phase = Phase::Play;
                return Tap::Skipped;
            }
            Phase::Cleared { at } => {
                // Straight on to the next level.
                self.phase = Phase::Cleared {
                    at: at.min(now.saturating_sub(CLEARED_MS)),
                };
                return Tap::Skipped;
            }
            Phase::Jammed { .. } => return Tap::Nothing,
            Phase::Play => {}
        }
        let Some(k) = stick.map(|k| k as usize) else {
            return Tap::Nothing;
        };
        let s = self.board.sticks[k];
        if s.out || s.moving.is_some() {
            return Tap::Nothing;
        }
        match self.board.blocked(k) {
            None => {
                self.board.take_off(k);
                self.board.sticks[k].moving = Some(Move::Exit { at: now });
                self.pick = 0;
                Tap::Slid
            }
            Some((by, steps)) => {
                self.board.sticks[k].moving = Some(Move::Bump { at: now, steps });
                self.flash = Some((by as u8, now + FLASH_MS));
                self.jams += 1;
                let last = self.jams >= JAMS;
                if last {
                    self.phase = Phase::Jammed { at: now };
                }
                Tap::Jammed { last }
            }
        }
    }

    /// Move the game on: finish slides and bumps, and step through the
    /// level number, "Clear!" and "Jammed".
    pub fn tick(&mut self, now: u64) -> Tick {
        for k in 0..self.board.count as usize {
            let done = match self.board.sticks[k].moving {
                Some(Move::Exit { at }) => exit_s(now.saturating_sub(at)) >= exit_path(&self.board, k).end,
                Some(Move::Bump { at, .. }) => now.saturating_sub(at) >= BUMP_MS,
                None => false,
            };
            if done {
                self.board.sticks[k].moving = None;
            }
        }
        if self.flash.is_some_and(|(_, until)| now >= until) {
            self.flash = None;
        }
        match self.phase {
            Phase::Ready => {
                self.phase = Phase::Intro { at: now };
                Tick::None
            }
            Phase::Intro { at } if now.saturating_sub(at) >= INTRO_MS => {
                self.phase = Phase::Play;
                Tick::None
            }
            Phase::Play if self.board.sticks().iter().all(Stick::gone) => {
                self.phase = Phase::Cleared { at: now };
                Tick::Cleared
            }
            Phase::Cleared { at } if now.saturating_sub(at) >= CLEARED_MS => Tick::NextLevel,
            Phase::Jammed { at } if now.saturating_sub(at) >= JAMMED_MS => {
                let level = self.level;
                *self = Self::new(self.board.n, level, self.seed);
                self.phase = Phase::Play;
                Tick::Restarted
            }
            _ => Tick::None,
        }
    }

    /// Whether anything is moving (for the sleep timer).
    pub fn busy(&self) -> bool {
        self.phase != Phase::Play || self.board.sticks().iter().any(|s| s.moving.is_some())
    }

    fn flashing(&self, k: usize, now: u64) -> bool {
        self.flash
            .is_some_and(|(f, until)| f as usize == k && now < until && (until - now) / 90 % 2 == 0)
    }
}

/// How far a sliding stick has gone `ms` after it set off, in doubled
/// cells: it picks up speed as it goes.
fn exit_s(ms: u64) -> f32 {
    let t = ms as f32 / 1000.0;
    14.0 * t + 60.0 * t * t
}

/// The points a stick's body runs through, tail to head, with a bend point
/// on each edge it crosses.
fn body_points(b: &Board, s: &Stick) -> heapless::Vec<F, { 2 * MAX_LEN }> {
    let mut pts = heapless::Vec::new();
    let cells = s.cells();
    let _ = pts.push(fv(cells[0]));
    for w in cells.windows(2) {
        let (fa, fb) = (face_of(w[0], b.n), face_of(w[1], b.n));
        if fa != fb {
            let normal = basis(fb).2;
            let _ = pts.push(fv(add(w[0], normal)));
        }
        let _ = pts.push(fv(w[1]));
    }
    pts
}

/// A path for a stick to slide along, and where its tail leaves the board.
struct Path {
    pts: heapless::Vec<F, { 2 * MAX_LEN + MAX_N as usize + 2 }>,
    /// The body's length.
    len: f32,
    /// How far the window must run for the whole stick to be off.
    end: f32,
}

fn exit_path(b: &Board, k: usize) -> Path {
    let s = &b.sticks[k];
    let body = body_points(b, s);
    let len = poly_len(&body);
    let mut pts: heapless::Vec<F, { 2 * MAX_LEN + MAX_N as usize + 2 }> = heapless::Vec::new();
    for p in body.iter() {
        let _ = pts.push(*p);
    }
    let lane = b.lane(s.head(), s.dir);
    for c in lane.iter() {
        let _ = pts.push(fv(*c));
    }
    let last = lane.last().copied().unwrap_or(s.head());
    let edge = fv(add(last, s.dir));
    let _ = pts.push(edge);
    let far = 2.0 * b.n as f32 + len + 4.0;
    let d = fv(s.dir);
    let _ = pts.push([0, 1, 2].map(|i| edge[i] + d[i] * far));
    // Off once the tail has passed the edge and a little more.
    let to_edge = poly_len(&pts[..pts.len() - 1]);
    Path {
        pts,
        len,
        end: to_edge + 1.2,
    }
}

fn poly_len(pts: &[F]) -> f32 {
    pts.windows(2).map(|w| fdist(w[0], w[1])).sum()
}

/// The part of `pts` from `a` to `b` along it.
fn window<const N: usize>(pts: &[F], a: f32, b: f32) -> heapless::Vec<F, N> {
    let mut out = heapless::Vec::new();
    let at = |s: f32| {
        let mut run = 0.0;
        for w in pts.windows(2) {
            let l = fdist(w[0], w[1]);
            if s <= run + l || l == 0.0 {
                return fmix(w[0], w[1], ((s - run) / l.max(1e-6)).clamp(0.0, 1.0));
            }
            run += l;
        }
        pts[pts.len() - 1]
    };
    let _ = out.push(at(a));
    let mut run = 0.0;
    for w in pts.windows(2) {
        run += fdist(w[0], w[1]);
        if run > a && run < b {
            let _ = out.push(w[1]);
        }
    }
    let _ = out.push(at(b));
    out
}

/// Where stick `k` is drawn now: its body, slid along its way out.
fn stick_points(b: &Board, k: usize, now: u64) -> heapless::Vec<F, 24> {
    let s = &b.sticks[k];
    match s.moving {
        None => {
            let mut out = heapless::Vec::new();
            for p in body_points(b, s).iter() {
                let _ = out.push(*p);
            }
            out
        }
        Some(Move::Exit { at }) => {
            let path = exit_path(b, k);
            let s0 = exit_s(now.saturating_sub(at));
            window(&path.pts, s0, s0 + path.len)
        }
        Some(Move::Bump { at, steps }) => {
            let path = exit_path(b, k);
            let t = (now.saturating_sub(at) as f32 / BUMP_MS as f32).clamp(0.0, 1.0);
            let s0 = (2.0 * steps as f32 + 0.6) * sinf(core::f32::consts::PI * t);
            window(&path.pts, s0, s0 + path.len)
        }
    }
}

// ---------- drawing ----------

/// How the board sits on a panel: the cell pitch and the stick width in
/// whole pixels, so sticks land on pixel edges.
#[derive(Clone, Copy, Debug)]
pub struct Grid {
    /// Pixels from one cell centre to the next (even).
    pub cell: f32,
    /// Stick width (even).
    pub width: f32,
    /// Half the panel.
    pub half: f32,
    /// Board cells across.
    pub n: u8,
}

impl Grid {
    /// The grid for an `n`×`n` board on a `side`-pixel panel, inside a
    /// margin of 4 px on 64×64 (the glass rounds the corners).
    pub fn new(n: u8, side: usize) -> Self {
        let margin = side / 16;
        let cell = ((side - 2 * margin) / n as usize) & !1;
        let width = ((cell * 2 / 5).div_ceil(2) * 2).max(2);
        Self {
            cell: cell as f32,
            width: width as f32,
            half: side as f32 / 2.0,
            n,
        }
    }

    /// A doubled die coordinate along one of a face's axes, in pixels from
    /// the panel centre. Inside the board it's linear; the last half cell
    /// before an edge also takes in the margin, so a stick that bends over
    /// an edge runs to the panel's edge and on into the next face.
    pub fn px(&self, u: f32) -> f32 {
        let n = self.n as f32;
        let a = u.abs();
        let half_cell = self.cell / 2.0;
        let edge = n * half_cell;
        let margin = self.half - edge;
        let r = if a <= n - 1.0 {
            a * half_cell
        } else if a <= n {
            (n - 1.0) * half_cell + (a - (n - 1.0)) * (half_cell + margin)
        } else {
            self.half + (a - n) * half_cell
        };
        r * u.signum()
    }
}

/// What the die knows about how it's held, for drawing.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub top: Face,
    pub now: u64,
}

/// One face of the board, in pixel units around the panel centre (x right,
/// y down), drawn upright for the die's own face axes, not the reader's:
/// the board is fixed to the die like a printed one.
pub fn draw_face<T: Target>(p: &mut Painter<T>, rush: &Rush, face: Face, look: Look) {
    if rush.phase != Phase::Play {
        return;
    }
    let b = &rush.board;
    let g = Grid::new(b.n, T::WIDTH);
    let (x, y, normal) = basis(face);
    let (fx, fy, fn_) = (fv(x), fv(y), fv(normal));
    let n = b.n as f32;
    let to_px = |q: F| (g.px(fdot(q, fx)), -g.px(fdot(q, fy)));
    let on = |q: F| (fdot(q, fn_) - n).abs() < 1e-3;
    let lit = rush.lit();
    let selected = rush.selected();
    let tipping = rush.aim.is_some() && look.top == face;
    let leaning = rush.aim.is_some();
    let blink = (look.now / 200) % 2 == 0;

    // Faint dots on the empty cells' centres.
    for i in 0..b.n {
        for j in 0..b.n {
            let u = (2 * i as i32 - b.n as i32 + 1) as f32;
            let v = (2 * j as i32 - b.n as i32 + 1) as f32;
            let (px, py) = (g.px(u), -g.px(v));
            p.fill_rect(px - 1.0, py - 1.0, 2.0, 2.0, Style::color(pal::FAINT, 1.0, 0.0));
        }
    }

    // The downhill edge of the top screen.
    if let (true, Some(d)) = (tipping, rush.downhill()) {
        let colour = if lit.is_empty() { pal::FAINT } else { pal::VIOLET };
        let (dx, dy) = (fdot(fv(d), fx), -fdot(fv(d), fy));
        let edge = n * g.cell / 2.0;
        let (w, h) = if dx != 0.0 {
            (2.0, 2.0 * edge)
        } else {
            (2.0 * edge, 2.0)
        };
        let (cx, cy) = (dx * (edge + 2.0), dy * (edge + 2.0));
        p.fill_rect(cx - w / 2.0, cy - h / 2.0, w, h, Style::color(colour, 1.0, 0.0));
    }

    for k in 0..b.count as usize {
        let s = &b.sticks[k];
        if s.gone() {
            continue;
        }
        let pts = stick_points(b, k, look.now);
        let mut colour = pal::WHITE;
        let mut alpha = 1.0;
        if leaning && !s.out && !lit.contains(&(k as u8)) {
            colour = pal::DIM;
        }
        if selected == Some(k as u8) {
            colour = pal::MINT;
            alpha = if blink { 1.0 } else { 0.55 };
        }
        if matches!(s.moving, Some(Move::Exit { .. })) {
            colour = pal::MINT;
        }
        if rush.flashing(k, look.now) {
            colour = pal::RED;
        }
        let style = Style::color(colour, alpha, 0.0);
        draw_stick(p, &g, &pts, &on, &to_px, style);
        // The arrowhead, on the head's face.
        let head = pts[pts.len() - 1];
        if on(head) && face_of_point(head, n) == Some(face) {
            let d = fv(s.dir);
            let (dx, dy) = (fdot(d, fx), -fdot(d, fy));
            let (hx, hy) = to_px(head);
            let len = g.cell / 2.0 - 1.0;
            let wing = g.width;
            let tip = (hx + dx * len, hy + dy * len);
            let back = (hx - dx, hy - dy);
            p.fill_triangle(
                [
                    tip,
                    (back.0 - dy * wing, back.1 + dx * wing),
                    (back.0 + dy * wing, back.1 - dx * wing),
                ],
                style,
            );
        }
    }
}

/// The face a point lies on, unless it's on an edge (or off the cube).
fn face_of_point(q: F, n: f32) -> Option<Face> {
    let hits: heapless::Vec<usize, 3> = (0..3).filter(|a| (q[*a].abs() - n).abs() < 1e-3).collect();
    if hits.len() != 1 {
        // An edge (two faces) is fine for the body; the head is never on one.
        return hits.first().map(|a| {
            let mut normal = [0i8; 3];
            normal[*a] = if q[*a] > 0.0 { 1 } else { -1 };
            face_of_normal(normal)
        });
    }
    let a = hits[0];
    let mut normal = [0i8; 3];
    normal[a] = if q[a] > 0.0 { 1 } else { -1 };
    Some(face_of_normal(normal))
}

/// A stick's segments on this face as axis-aligned bars, square-jointed.
fn draw_stick<T: Target>(
    p: &mut Painter<T>,
    g: &Grid,
    pts: &[F],
    on: &impl Fn(F) -> bool,
    to_px: &impl Fn(F) -> (f32, f32),
    style: Style,
) {
    let h = g.width / 2.0;
    for w in pts.windows(2) {
        if !(on(w[0]) && on(w[1])) {
            continue;
        }
        let (a, b) = (to_px(w[0]), to_px(w[1]));
        let snap = |v: f32| floorf(v + 0.5);
        let (x0, x1) = (snap(a.0.min(b.0)) - h, snap(a.0.max(b.0)) + h);
        let (y0, y1) = (snap(a.1.min(b.1)) - h, snap(a.1.max(b.1)) + h);
        p.fill_rect(x0, y0, x1 - x0, y1 - y0, style);
    }
    if pts.len() == 1 && on(pts[0]) {
        let (x, y) = to_px(pts[0]);
        p.fill_rect(x - h, y - h, g.width, g.width, style);
    }
}

/// What the screens say instead of the board between levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Banner {
    /// The level about to start.
    Level(u16),
    /// The level just cleared.
    Cleared(u16),
    /// The last jam: the board starts again.
    Jammed,
}

impl Banner {
    /// Its colour on the 64×64 die: mint for good news, red for the jam.
    pub fn colour(self) -> Color {
        match self {
            Banner::Level(_) => pal::WHITE,
            Banner::Cleared(_) => pal::MINT,
            Banner::Jammed => pal::RED,
        }
    }
}

impl Rush {
    /// The banner up now, and how long it has been up (seconds).
    pub fn banner(&self, now: u64) -> Option<(Banner, f32)> {
        let since = |at: u64| now.saturating_sub(at) as f32 / 1000.0;
        match self.phase {
            Phase::Ready => Some((Banner::Level(self.level), 0.0)),
            Phase::Intro { at } => Some((Banner::Level(self.level), since(at))),
            Phase::Cleared { at } => Some((Banner::Cleared(self.level), since(at))),
            Phase::Jammed { at } => Some((Banner::Jammed, since(at))),
            Phase::Play => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    /// Clear a board greedily: any free stick, until none are left.
    fn solvable(mut b: Board) -> bool {
        loop {
            let free = (0..b.count as usize).find(|k| !b.sticks[*k].out && b.blocked(*k).is_none());
            match free {
                Some(k) => b.take_off(k),
                None => return b.left() == 0,
            }
        }
    }

    #[test]
    fn every_board_can_be_cleared() {
        for n in [2, 3, 4] {
            for seed in 0..400 {
                let b = Board::generate(n, seed, 2 + (seed as usize % 5), 85);
                assert!(b.count > 0, "n={n} seed={seed} has sticks");
                assert!(solvable(b), "n={n} seed={seed}");
            }
        }
    }

    #[test]
    fn sticks_hold_their_own_cells_and_nothing_overlaps() {
        let b = Board::generate(4, 7, 6, 85);
        let mut seen = Vec::new();
        for (k, s) in b.sticks().iter().enumerate() {
            assert!(s.cells().len() >= MIN_LEN);
            for c in s.cells() {
                assert_eq!(b.at(*c), Some(k));
                assert!(!seen.contains(c));
                seen.push(*c);
            }
            // Neighbouring cells touch: one step apart, maybe over an edge.
            for w in s.cells().windows(2) {
                let ok = [[1, 0, 0], [0, 1, 0], [0, 0, 1]]
                    .into_iter()
                    .flat_map(|a| [a, neg(a)])
                    .any(|d| b.step(w[0], d).0 == w[1]);
                assert!(ok, "{w:?} adjacent");
            }
            // The head points along its own face.
            let normal = basis(b.head_face(k)).2;
            assert_eq!(dot(s.dir, normal), 0);
        }
        // It's full-ish.
        assert!(seen.len() * 100 >= MAX_CELLS * 60, "{} cells", seen.len());
    }

    #[test]
    fn the_same_seed_builds_the_same_board() {
        assert_eq!(Board::generate(3, 42, 4, 85), Board::generate(3, 42, 4, 85));
        assert_ne!(Board::generate(3, 42, 4, 85), Board::generate(3, 43, 4, 85));
    }

    #[test]
    fn stepping_over_an_edge_turns_onto_the_next_face() {
        let b = Board::empty(3);
        // Top face (+Y), the cell nearest +X, going +X.
        let c = [2, 3, 0];
        let (next, going) = b.step(c, [1, 0, 0]);
        assert_eq!(next, [3, 2, 0]);
        assert_eq!(face_of(next, 3), Face::PosX);
        assert_eq!(going, [0, -1, 0]);
        assert_eq!(b.lane([-2, 3, 0], [1, 0, 0]).as_slice(), &[[0, 3, 0], [2, 3, 0]]);
    }

    fn play(rush: &mut Rush) {
        rush.phase = Phase::Play;
    }

    /// The top face and downhill direction that light stick `k`.
    fn aim_at(rush: &mut Rush, k: usize) {
        let top = rush.board.head_face(k);
        let d = rush.board.sticks()[k].dir;
        rush.aim(top, Some(d));
        while rush.selected() != Some(k as u8) {
            assert_eq!(rush.next_pick(), Tap::Picked);
        }
    }

    #[test]
    fn leaning_lights_the_top_sticks_that_point_downhill() {
        let mut rush = Rush::new(3, 4, 9);
        play(&mut rush);
        assert!(rush.lit().is_empty(), "level: nothing lit");
        let k = 0;
        let top = rush.board.head_face(k);
        let d = rush.board.sticks()[k].dir;
        assert!(rush.aim(top, Some(d)));
        let lit = rush.lit();
        assert!(lit.contains(&(k as u8)));
        for s in lit.iter() {
            assert_eq!(rush.board.head_face(*s as usize), top);
            assert_eq!(rush.board.sticks()[*s as usize].dir, d);
        }
        // Nearest the edge first.
        let lanes: Vec<usize> = lit
            .iter()
            .map(|s| rush.board.lane(rush.board.sticks()[*s as usize].head(), d).len())
            .collect();
        assert!(lanes.windows(2).all(|w| w[0] <= w[1]));
        assert!(!rush.aim(top, Some(d)), "the same lean again changes nothing");
        // Back to level: the pick stays, so the tap can come after.
        rush.aim(top, None);
        assert_eq!(rush.downhill(), Some(d));
        assert_eq!(rush.lean(), None);
        assert!(rush.selected().is_some());
        // Leaning another way picks again...
        rush.aim(top, Some(neg(d)));
        assert_eq!(rush.downhill(), Some(neg(d)));
        // ...and turning another face up drops it.
        rush.aim(top.opposite(), None);
        assert_eq!(rush.downhill(), None);
        assert_eq!(rush.selected(), None);
    }

    #[test]
    fn a_clear_stick_slides_off_and_a_blocked_one_jams() {
        let mut rush = Rush::new(4, 6, 3);
        play(&mut rush);
        let free = (0..rush.board.count as usize)
            .find(|k| rush.board.blocked(*k).is_none())
            .unwrap();
        aim_at(&mut rush, free);
        let before = rush.board.left();
        assert_eq!(rush.slide(rush.selected(), 100), Tap::Slid);
        assert_eq!(rush.board.left(), before - 1);
        assert!(rush.board.sticks()[free].moving.is_some(), "still sliding");
        rush.tick(5_000);
        assert!(rush.board.sticks()[free].gone());

        let stuck = (0..rush.board.count as usize)
            .find(|k| !rush.board.sticks()[*k].out && rush.board.blocked(*k).is_some())
            .expect("a blocked stick");
        aim_at(&mut rush, stuck);
        assert_eq!(rush.slide(rush.selected(), 6_000), Tap::Jammed { last: false });
        assert_eq!(rush.jams, 1);
        assert_eq!(rush.board.left(), before - 1, "a jam moves nothing");
    }

    #[test]
    fn three_jams_start_the_board_again() {
        let mut rush = Rush::new(4, 6, 3);
        play(&mut rush);
        let fresh = rush.board;
        let stuck = (0..rush.board.count as usize)
            .find(|k| rush.board.blocked(*k).is_some())
            .unwrap();
        aim_at(&mut rush, stuck);
        let mut t = 0;
        for i in 0..JAMS {
            t += 1_000;
            rush.tick(t);
            let last = i + 1 == JAMS;
            assert_eq!(rush.slide(Some(stuck as u8), t), Tap::Jammed { last });
        }
        assert!(matches!(rush.phase, Phase::Jammed { .. }));
        assert_eq!(rush.tick(t + JAMMED_MS), Tick::Restarted);
        assert_eq!(rush.board, fresh);
        assert_eq!(rush.jams, 0);
        assert_eq!(rush.phase, Phase::Play);
    }

    #[test]
    fn clearing_every_stick_wins_the_level() {
        let mut rush = Rush::new(3, 1, 11);
        assert_eq!(rush.tick(100), Tick::None, "on screen: the level number");
        assert_eq!(rush.phase, Phase::Intro { at: 100 });
        rush.tick(100 + INTRO_MS);
        assert_eq!(rush.phase, Phase::Play);
        let mut t = 100 + INTRO_MS;
        while rush.board.left() > 0 {
            let k = (0..rush.board.count as usize)
                .find(|k| !rush.board.sticks()[*k].out && rush.board.blocked(*k).is_none())
                .unwrap();
            aim_at(&mut rush, k);
            assert_eq!(rush.slide(rush.selected(), t), Tap::Slid);
            t += 2_000;
            rush.tick(t);
        }
        assert!(matches!(rush.phase, Phase::Cleared { .. }), "{:?}", rush.phase);
        assert_eq!(rush.tick(t + CLEARED_MS), Tick::NextLevel);
        rush.next_level(5);
        assert_eq!(rush.level, 2);
        assert_eq!(rush.phase, Phase::Ready);
        assert!(rush.board.left() > 0);
    }

    #[test]
    fn grids_land_on_whole_pixels() {
        for (n, side, cell, width) in [
            (3, 64, 18.0, 8.0),
            (4, 64, 14.0, 6.0),
            (3, 96, 28.0, 12.0),
            (4, 96, 20.0, 8.0),
        ] {
            let g = Grid::new(n, side);
            assert_eq!((g.cell, g.width), (cell, width), "n={n} on {side}");
            // Edge points reach the panel's edge.
            assert_eq!(g.px(n as f32), side as f32 / 2.0);
            assert_eq!(g.px(-(n as f32)), -(side as f32) / 2.0);
            // Cell centres sit on whole pixels.
            for i in 0..n {
                let u = (2 * i as i32 - n as i32 + 1) as f32;
                assert_eq!(g.px(u), roundf(g.px(u)));
            }
        }
    }
}
