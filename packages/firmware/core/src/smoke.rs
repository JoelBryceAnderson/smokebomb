//! Smoke: the live particle system (SIM_SPEC part D, decision H1).
//!
//! Particles live on the surface of the unit cube, each face ±1 on its own
//! axis, and slide over it: gravity pulls them "downhill", curl noise stirs
//! them, and a particle that crosses an edge carries on over the next face.
//! Each frame they're stamped additively onto the faces with round sprites
//! from the asset pack, and a particle near an edge is also stamped onto the
//! neighbouring face, so the smoke wraps round the die continuously.
//!
//! This is a statement-for-statement port of the mockup, including the order
//! it draws random numbers in, so a scenario started from the same generator
//! state (the capture tool seeds the mockup's `Math.random` with mulberry32)
//! plays out the same way.

use heapless::Vec;
use libm::{cosf, powf, sinf, sqrtf};
use smokebomb_hal::{AssetStore, Face, PANEL_HEIGHT, PANEL_WIDTH};
use smokebomb_shared::assets::{
    SectionKind, SpriteEntry, SpriteKind, SPRITES_HEADER_LEN, SPRITE_ENTRY_LEN, SPRITE_PROFILE_LEN,
};

use crate::display::Framebuffer;
use crate::gfx::{CENTER, K};
use crate::orientation::BASES;
use crate::pack::PackIndex;

/// Room for a full shake (380), the landing top-up (76) and embers (36),
/// with some to spare for a boot burst still draining.
pub const MAX_PARTICLES: usize = 640;

/// Held smoke at full charge, and the landing counts (SIM_SPEC D3).
const FULL: usize = 380;
const FULL_REDUCED: usize = 150;
const EMBERS: usize = 36;
const EMBERS_REDUCED: usize = 10;
const BURST: usize = 170;
const BURST_REDUCED: usize = 60;
const GOLD: usize = 120;
const GOLD_REDUCED: usize = 40;
const FIZZLE: usize = 36;
const FIZZLE_REDUCED: usize = 12;
/// Held smoke added per frame while shaking, at most.
const SHAKE_SPAWN_PER_FRAME: usize = 14;
/// Timing slack at a boundary frame (s): well under a frame, well over f32
/// rounding at these magnitudes.
const TIME_EPS: f32 = 1e-4;
/// Shaking charges the smoke up over this long (s).
const CHARGE_S: f32 = 1.6;

/// The mockup's seeded `Math.random`: mulberry32.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SmokeRng(u32);

impl SmokeRng {
    pub const fn new(seed: u32) -> Self {
        Self(seed)
    }

    /// Uniform in [0, 1), exactly as the capture tool's `Math.random`.
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (t ^ (t >> 14)) as f64 / 4_294_967_296.0
    }

    fn r(&mut self) -> f32 {
        self.next_f64() as f32
    }

    /// `(Math.random() * n) | 0`.
    fn index(&mut self, n: usize) -> usize {
        ((self.next_f64() * n as f64) as usize).min(n - 1)
    }

    pub fn skip(&mut self, n: u32) {
        for _ in 0..n {
            self.next_f64();
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    p: [f32; 3],
    v: [f32; 3],
    face: Face,
    kind: SpriteKind,
    /// Age and lifetime (s).
    life: f32,
    max: f32,
    /// Sprite radius in canvas units.
    size: f32,
    seed: f32,
    /// Held in the hand: banked over the face centres and not ageing.
    hold: bool,
    /// Banked like held smoke until this time (s); −∞ for none.
    linger: f32,
}

/// Samples of a sprite's opacity by squared distance from its centre.
const BY_D2: usize = 1024;

#[derive(Clone, Copy)]
struct Sprite {
    value: u8,
    profile: [u8; SPRITE_PROFILE_LEN],
    /// Opacity (0–255) at squared radius `i / BY_D2` of the full radius
    /// squared, so stamping needs no square roots.
    by_d2: [u8; BY_D2],
}

impl Sprite {
    fn new(value: u8, profile: [u8; SPRITE_PROFILE_LEN]) -> Self {
        let steps = (SPRITE_PROFILE_LEN - 1) as f32;
        let by_d2 = core::array::from_fn(|i| {
            // Centre of the bucket, in radius.
            let t = sqrtf((i as f32 + 0.5) / BY_D2 as f32) * steps;
            let j = (t as usize).min(SPRITE_PROFILE_LEN - 2);
            let f = t - j as f32;
            let a = profile[j] as f32 + (profile[j + 1] as f32 - profile[j] as f32) * f;
            (a + 0.5) as u8
        });
        Self {
            value,
            profile,
            by_d2,
        }
    }
}

/// Where the smoke is in the throw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Calm,
    Shaking,
    /// From the throw until it lands.
    Tumbling,
}

/// What a special result adds once the smoke has cleared (SIM_SPEC C6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Special {
    /// Every die showed its top value: a gold ring.
    Max,
    /// Every die showed 1: a grey fizzle.
    Dud,
}

pub struct Smoke {
    particles: Vec<Particle, MAX_PARTICLES>,
    sprites: [Option<Sprite>; 4],
    rng: SmokeRng,
    phase: Phase,
    charge: f32,
    /// Gravity "up" in die coordinates (unit length).
    up: [f32; 3],
    reduced: bool,
    /// The smoke's own clock (s): the sum of its steps, like the mockup's
    /// `now`, so lingering and the curl noise run on the same time base.
    time: f32,
}

impl Smoke {
    /// Load the sprites from the pack. Without them the die simulates smoke
    /// but draws none.
    pub fn load<A: AssetStore>(assets: &mut A, pack: &PackIndex, seed: u32) -> Self {
        let mut sprites = [None; 4];
        if let Some(section) = pack.sections(SectionKind::Sprites).next() {
            let mut hdr = [0u8; SPRITES_HEADER_LEN];
            if assets.read(section.offset, &mut hdr).is_ok() {
                let count = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                for i in 0..count {
                    let mut e = [0u8; SPRITE_ENTRY_LEN];
                    let off = section.offset + (SPRITES_HEADER_LEN + i * SPRITE_ENTRY_LEN) as u32;
                    if assets.read(off, &mut e).is_err() {
                        break;
                    }
                    let entry = SpriteEntry::decode(&e);
                    if let Some(slot) = sprites.get_mut(entry.kind as usize) {
                        *slot = Some(Sprite::new(entry.value, entry.profile));
                    }
                }
            }
        }
        Self {
            particles: Vec::new(),
            sprites,
            rng: SmokeRng::new(seed),
            phase: Phase::Calm,
            charge: 0.0,
            up: [0.0, 1.0, 0.0],
            reduced: false,
            time: 0.0,
        }
    }

    /// Replace the random generator (tests replay the mockup's sequence).
    pub fn set_rng(&mut self, rng: SmokeRng) {
        self.rng = rng;
    }

    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced = on;
    }

    /// Particles of one kind.
    pub fn count(&self, kind: SpriteKind) -> usize {
        self.particles.iter().filter(|q| q.kind == kind).count()
    }

    pub fn len(&self) -> usize {
        self.particles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    /// Any smoke or embers left (specials wait for them to clear).
    pub fn has_smoke(&self) -> bool {
        self.particles
            .iter()
            .any(|q| matches!(q.kind, SpriteKind::Smoke | SpriteKind::Ember))
    }

    pub fn clear(&mut self) {
        self.particles.clear();
        self.phase = Phase::Calm;
        self.charge = 0.0;
    }

    /// Gravity up in die coordinates (any length).
    pub fn set_up(&mut self, up: [f32; 3]) {
        let l = sqrtf(dot(up, up));
        if l > 1e-6 {
            self.up = up.map(|u| u / l);
        }
    }

    fn full(&self) -> usize {
        if self.reduced {
            FULL_REDUCED
        } else {
            FULL
        }
    }

    // ---------- the throw (SIM_SPEC C5) ----------

    /// The die starts shaking in the hand: loose smoke goes, held smoke
    /// builds from nothing.
    pub fn shake_start(&mut self) {
        if self.phase != Phase::Shaking {
            self.particles.retain(|q| q.hold);
            self.charge = 0.0;
        }
        self.phase = Phase::Shaking;
    }

    /// Thrown: the held smoke rides along, topped up to a full cloud.
    pub fn throw(&mut self) {
        self.particles.retain(|q| q.hold);
        let full = self.full();
        let held = self.particles.len();
        if held < full {
            let before = self.particles.len();
            self.spawn(SpriteKind::Smoke, full - held, Where::All);
            for q in &mut self.particles[before..] {
                q.hold = true;
                q.life = 0.0;
            }
        }
        self.charge = 1.0;
        self.phase = Phase::Tumbling;
    }

    pub fn shaking(&self) -> bool {
        self.phase == Phase::Shaking
    }

    pub fn tumbling(&self) -> bool {
        self.phase == Phase::Tumbling
    }

    /// Landed: the held smoke drains, lingering over the faces for a moment,
    /// with a top-up and a scatter of embers.
    pub fn land(&mut self) {
        let now = self.time;
        for i in 0..self.particles.len() {
            if self.particles[i].hold {
                let max = 2.0 + self.rng.r() * 1.4;
                let q = &mut self.particles[i];
                q.hold = false;
                q.life = 0.0;
                q.max = max;
                q.linger = now + 0.6;
            }
        }
        let before = self.particles.len();
        let top_up = libm::roundf(self.full() as f32 * (1.0 - 0.8 * self.charge)) as usize;
        self.spawn(SpriteKind::Smoke, top_up, Where::All);
        for q in &mut self.particles[before..] {
            q.linger = now + 0.6;
        }
        let embers = if self.reduced { EMBERS_REDUCED } else { EMBERS };
        self.spawn(SpriteKind::Ember, embers, Where::All);
        self.charge = 0.0;
        self.phase = Phase::Calm;
    }

    /// The top face's boot burst: a thick cloud that hangs over the middle
    /// of the screen for a moment, then rolls off the edges (SIM_SPEC C1).
    pub fn burst(&mut self, face: Face) {
        let now = self.time;
        let (axis, sign) = axis_sign(face);
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        let n = if self.reduced { BURST_REDUCED } else { BURST };
        for _ in 0..n {
            let th = self.rng.r() * core::f32::consts::TAU;
            let mut dir = [0.0; 3];
            dir[a1] = cosf(th);
            dir[a2] = sinf(th);
            let mut p = [0.0; 3];
            p[axis] = sign;
            let r = 0.03 + self.rng.r() * 0.35;
            let p = add(p, scale(dir, r));
            let v = scale(dir, 0.3 + self.rng.r() * 0.6);
            let max = 1.9 + self.rng.r() * 0.9;
            let size = 22.0 + self.rng.r() * 18.0;
            let seed = self.rng.r() * 10.0;
            self.push(Particle {
                p,
                v,
                face,
                kind: SpriteKind::Smoke,
                life: 0.0,
                max,
                size,
                seed,
                hold: false,
                linger: now + 0.75,
            });
        }
    }

    /// A special result's effect, once the smoke has cleared.
    pub fn special(&mut self, special: Special) {
        match special {
            Special::Max => {
                let n = if self.reduced { GOLD_REDUCED } else { GOLD };
                self.spawn(SpriteKind::Gold, n, Where::Ring);
            }
            Special::Dud => {
                let n = if self.reduced { FIZZLE_REDUCED } else { FIZZLE };
                self.spawn(SpriteKind::Fizzle, n, Where::Top);
            }
        }
    }

    // ---------- per frame (SIM_SPEC D2) ----------

    /// Advance one frame of `dt` seconds. Shaking adds held smoke first;
    /// putting the die down without a throw lets it go.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.min(0.05);
        self.time += dt;
        let now = self.time;
        let mut agitate = 0.0;
        let mut slosh = [0.0; 3];
        match self.phase {
            Phase::Shaking => {
                self.charge = (self.charge + dt / CHARGE_S).min(1.0);
                agitate = 3.0 + self.charge * 3.0;
                // The mockup sloshes in world space; the die only knows its
                // own axes, which serve as well while it's being shaken.
                slosh = [
                    sinf(now * 17.0) * 2.5,
                    cosf(now * 13.0) * 2.5,
                    sinf(now * 11.0 + 1.0) * 2.5,
                ];
                let target = libm::roundf(self.full() as f32 * self.charge) as usize;
                let held = self.particles.iter().filter(|q| q.hold).count();
                if held < target {
                    let before = self.particles.len();
                    self.spawn(
                        SpriteKind::Smoke,
                        (target - held).min(SHAKE_SPAWN_PER_FRAME),
                        Where::All,
                    );
                    for q in &mut self.particles[before..] {
                        q.hold = true;
                    }
                }
            }
            Phase::Tumbling => agitate = 5.0,
            Phase::Calm => {
                if self.particles.iter().any(|q| q.hold) {
                    // Put down without throwing: the smoke settles and fades.
                    for q in &mut self.particles {
                        if q.hold {
                            q.hold = false;
                            q.life = q.max * 0.35;
                        }
                    }
                    self.charge = 0.0;
                }
            }
        }

        let up = self.up;
        let top = face_of(up);
        let rng = &mut self.rng;
        self.particles.retain_mut(|q| {
            q.life += dt;
            if q.hold {
                q.life = q.life.min(q.max * 0.2);
            }
            if q.life > q.max {
                return false;
            }
            let n = normal(q.face);
            // "At least" the linger: the mockup's clock is a sum of frame
            // times that falls just short on the boundary frame, so its
            // lingering smoke gets that frame too.
            let lingering = now < q.linger + TIME_EPS;
            let buoy = if q.hold {
                -0.35
            } else if lingering {
                -0.25
            } else {
                match q.kind {
                    SpriteKind::Fizzle | SpriteKind::Gold => 0.0,
                    SpriteKind::Ember => -0.8,
                    SpriteKind::Smoke => -1.3,
                }
            };
            if (q.hold || lingering) && q.kind == SpriteKind::Smoke {
                // Banked over the centre of each screen, where the number
                // will appear.
                let t = tangential(q.p, n);
                q.v = add(q.v, scale(t, -2.4 * dt));
            }
            if q.hold && agitate > 0.0 {
                for c in 0..3 {
                    q.v[c] += (rng.r() - 0.5) * agitate * dt * 6.0;
                }
                q.v = add(q.v, scale(slosh, agitate * dt));
            }
            q.v = add(q.v, scale(up, buoy * dt));
            if !q.hold && q.face == top && !matches!(q.kind, SpriteKind::Gold | SpriteKind::Fizzle) {
                // On the top face the cloud spreads to the edges and pours
                // over them.
                let t = tangential(q.p, n);
                let l2 = dot(t, t);
                if l2 > 1e-6 {
                    q.v = add(q.v, scale(t, 0.9 * dt / sqrtf(l2)));
                }
            }
            if q.kind == SpriteKind::Gold {
                let target = scale(normalize(cross(up, q.p)), 1.6);
                let k = (2.5 * dt).min(1.0);
                q.v = add(q.v, scale(sub(target, q.v), k));
            } else {
                let (s, t) = (q.seed, now);
                q.v[0] += sinf(3.3 * q.p[1] + 1.7 * t + s) * 0.9 * dt;
                q.v[1] += sinf(3.1 * q.p[2] + 1.9 * t + s * 1.3) * 0.9 * dt;
                q.v[2] += sinf(2.9 * q.p[0] + 1.5 * t + s * 0.7) * 0.9 * dt;
                let damp = if q.kind == SpriteKind::Fizzle {
                    3.0
                } else if q.hold {
                    1.6
                } else {
                    0.7
                };
                q.v = scale(q.v, 1.0 - damp * dt);
            }
            q.v = tangential(q.v, n);
            q.p = project(add(q.p, scale(q.v, dt)));
            let nf = face_of(q.p);
            if nf != q.face {
                let nn = normal(nf);
                let c = dot(q.v, nn);
                q.v = sub(sub(q.v, scale(nn, c)), scale(n, c));
                q.face = nf;
            }
            true
        });
    }

    // ---------- spawning (SIM_SPEC D3) ----------

    fn push(&mut self, q: Particle) {
        let _ = self.particles.push(q);
    }

    fn spawn(&mut self, kind: SpriteKind, n: usize, place: Where) {
        let uf = face_of(self.up);
        let (ua, us) = axis_sign(uf);
        let down = uf.opposite();
        let ember = kind == SpriteKind::Ember;
        for _ in 0..n {
            let rng = &mut self.rng;
            let q = match place {
                Where::All => {
                    // Any face but the one facing down, bunched to the middle.
                    let not_down: Vec<Face, 5> = Face::ALL.into_iter().filter(|&f| f != down).collect();
                    let f = not_down[rng.index(not_down.len())];
                    let (axis, sign) = axis_sign(f);
                    let mut p = [0.0; 3];
                    p[axis] = sign;
                    let mut bunched = || (rng.r() + rng.r() + rng.r() - 1.5) / 1.5;
                    p[(axis + 1) % 3] = bunched() * 0.85;
                    p[(axis + 2) % 3] = bunched() * 0.85;
                    let v = [rng.r() - 0.5, rng.r() - 0.5, rng.r() - 0.5].map(|c| c * 0.5);
                    let max = if ember {
                        0.7 + rng.r() * 0.6
                    } else {
                        2.2 + rng.r() * 1.4
                    };
                    let size = if ember {
                        7.0 + rng.r() * 7.0
                    } else {
                        26.0 + rng.r() * 20.0
                    };
                    Particle::new(p, v, kind, max, size, rng.r() * 10.0)
                }
                Where::Top => {
                    let n = normal(uf);
                    let (a1, a2) = ((ua + 1) % 3, (ua + 2) % 3);
                    let mut bunched = || (rng.r() + rng.r() + rng.r() - 1.5) / 1.5;
                    let mut p = [0.0; 3];
                    p[ua] = us;
                    p[a1] = bunched() * 0.95;
                    p[a2] = bunched() * 0.95;
                    let mut v = tangential(p, n);
                    if dot(v, v) < 1e-4 {
                        v[a1] = 0.1;
                    }
                    let speed = if ember {
                        0.6 + rng.r() * 0.8
                    } else {
                        0.15 + rng.r() * 0.45
                    };
                    let v = scale(normalize(v), speed);
                    let max = if ember {
                        0.7 + rng.r() * 0.6
                    } else {
                        2.4 + rng.r() * 1.4
                    };
                    let size = if ember {
                        7.0 + rng.r() * 7.0
                    } else {
                        26.0 + rng.r() * 20.0
                    };
                    Particle::new(p, v, kind, max, size, rng.r() * 10.0)
                }
                Where::Ring => {
                    // Side faces, near mid-height.
                    let sides: Vec<Face, 4> =
                        Face::ALL.into_iter().filter(|&f| axis_sign(f).0 != ua).collect();
                    let f = sides[rng.index(sides.len())];
                    let (axis, sign) = axis_sign(f);
                    let other = 3 - axis - ua;
                    let mut p = [0.0; 3];
                    p[axis] = sign;
                    p[ua] = (rng.r() - 0.5) * 0.35;
                    p[other] = (rng.r() * 2.0 - 1.0) * 0.95;
                    let gold = kind == SpriteKind::Gold;
                    let v = if gold {
                        scale(normalize(cross(self.up, p)), 1.6)
                    } else {
                        let mut v = scale(
                            self.up,
                            if ember {
                                0.9 + rng.r()
                            } else {
                                0.35 + rng.r() * 0.5
                            },
                        );
                        v[other] += (rng.r() - 0.5) * 0.9;
                        v
                    };
                    let max = if ember {
                        0.9 + rng.r() * 0.7
                    } else if gold {
                        2.0 + rng.r() * 0.8
                    } else {
                        2.2 + rng.r() * 1.3
                    };
                    let size = if ember {
                        8.0 + rng.r() * 8.0
                    } else if gold {
                        12.0 + rng.r() * 10.0
                    } else {
                        20.0 + rng.r() * 18.0
                    };
                    Particle::new(p, v, kind, max, size, rng.r() * 10.0)
                }
            };
            self.push(q);
        }
    }

    // ---------- drawing (SIM_SPEC D1) ----------

    /// Stamp every particle onto the faces, over whatever they show.
    pub fn draw(&self, frames: &mut [Framebuffer; 6]) {
        for q in &self.particles {
            let Some(sprite) = &self.sprites[q.kind as usize] else {
                continue;
            };
            let life_t = q.life / q.max;
            let fade_in = (q.life / 0.12).min(1.0);
            let alpha = if q.kind == SpriteKind::Gold {
                powf(sinf(core::f32::consts::PI * life_t).max(0.0), 0.8)
            } else {
                fade_in * powf((1.0 - life_t).max(0.0), 0.8)
            } * match q.kind {
                SpriteKind::Smoke => 0.55,
                SpriteKind::Fizzle => 0.3,
                _ => 0.9,
            };
            let grows = matches!(q.kind, SpriteKind::Smoke | SpriteKind::Fizzle);
            let size = q.size * (1.0 + if grows { life_t * 1.4 } else { 0.0 });
            stamp(&mut frames[q.face.index()], sprite, q.face, q.p, size, alpha);
            // Near an edge, also on the neighbouring face, as if the cube
            // were unfolded there.
            let (own_axis, own_sign) = axis_sign(q.face);
            let ru = size / 128.0;
            for g in Face::ALL {
                let (ga, gs) = axis_sign(g);
                if ga == own_axis {
                    continue;
                }
                let d = 1.0 - gs * q.p[ga];
                if d < ru {
                    let mut t = q.p;
                    t[own_axis] = own_sign * (1.0 + d);
                    t[ga] = gs;
                    stamp(&mut frames[g.index()], sprite, g, t, size, alpha);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Where {
    All,
    Top,
    Ring,
}

impl Particle {
    fn new(p: [f32; 3], v: [f32; 3], kind: SpriteKind, max: f32, size: f32, seed: f32) -> Self {
        Self {
            p,
            v,
            face: face_of(p),
            kind,
            life: 0.0,
            max,
            size,
            seed,
            hold: false,
            linger: f32::NEG_INFINITY,
        }
    }
}

/// Stamp one sprite centred on `p` (cube coordinates) on `face`, additively.
fn stamp(fb: &mut Framebuffer, sprite: &Sprite, face: Face, p: [f32; 3], radius: f32, alpha: f32) {
    let peak = sprite.value as f32 * alpha;
    if peak < 0.5 || radius <= 0.0 {
        return;
    }
    // Face-local position → canvas units (the face canvas spans ±128) → px.
    let b = &BASES[face.index()];
    let (cx, cy) = (dot(p, b.x) * 128.0, -dot(p, b.y) * 128.0);
    let (px, py, r) = (CENTER + cx * K, CENTER + cy * K, radius * K);
    // Only the part of a faint stamp that adds at least half a level to a
    // pixel shows; the profile falls with radius, so skip the rest.
    let visible = sprite
        .profile
        .iter()
        .rposition(|&a| peak * a as f32 / 255.0 >= 0.5)
        .map_or(0, |i| i + 1);
    if visible == 0 {
        return;
    }
    let reach = r * (visible.min(SPRITE_PROFILE_LEN - 1) as f32 / (SPRITE_PROFILE_LEN - 1) as f32);
    let y0 = libm::floorf(py - reach).max(0.0) as usize;
    let y1 = (libm::ceilf(py + reach).max(0.0) as usize).min(PANEL_HEIGHT);
    let reach2 = reach * reach;
    let per_d2 = BY_D2 as f32 / (r * r);
    let scale = peak / 255.0;
    for y in y0..y1 {
        let dy = y as f32 + 0.5 - py;
        let rem = reach2 - dy * dy;
        if rem <= 0.0 {
            continue;
        }
        // This row's span inside the stamp: pixel centres within `half`.
        let half = sqrtf(rem);
        let x0 = libm::ceilf(px - half - 0.5).max(0.0) as usize;
        let x1 = ((libm::floorf(px + half - 0.5) + 1.0).max(0.0) as usize).min(PANEL_WIDTH);
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - px;
            let i = (((dx * dx + dy * dy) * per_d2) as usize).min(BY_D2 - 1);
            let v = sprite.by_d2[i] as f32 * scale;
            if v >= 0.5 {
                fb.add_pixel(x, y, (v + 0.5) as u8);
            }
        }
    }
}

// ---------- cube geometry ----------

fn axis_sign(face: Face) -> (usize, f32) {
    let i = face.index();
    (i / 2, if i % 2 == 0 { 1.0 } else { -1.0 })
}

fn normal(face: Face) -> [f32; 3] {
    let (axis, sign) = axis_sign(face);
    let mut n = [0.0; 3];
    n[axis] = sign;
    n
}

/// The face a point on (or near) the cube surface is on: the mockup's
/// `surfaceFace`, ties going to the lower axis and zero counting as +.
fn face_of(p: [f32; 3]) -> Face {
    let a = p.map(libm::fabsf);
    let axis = if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    };
    Face::ALL[axis * 2 + if p[axis] < 0.0 { 1 } else { 0 }]
}

/// Back onto the cube surface: divide by the largest component.
fn project(p: [f32; 3]) -> [f32; 3] {
    let m = p.map(libm::fabsf).into_iter().fold(0.0, f32::max);
    if m > 0.0 {
        p.map(|c| c / m)
    } else {
        p
    }
}

/// `v` without its component along the unit normal `n`.
fn tangential(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    sub(v, scale(n, dot(v, n)))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|c| c * k)
}

/// Unit vector; zero stays zero (three.js `normalize`).
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = sqrtf(dot(a, a));
    if l > 0.0 {
        scale(a, 1.0 / l)
    } else {
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_matches_the_capture_tools_mulberry32() {
        // First values of mulberry32(42), as the capture tool's Math.random.
        let mut r = SmokeRng::new(42);
        let got: [f64; 3] = core::array::from_fn(|_| r.next_f64());
        let want = [0.6011037519201636, 0.44829055899754167, 0.8524657934904099];
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1e-12, "{got:?}");
        }
    }

    fn smoke() -> Smoke {
        let mut s = Smoke {
            particles: Vec::new(),
            sprites: [None; 4],
            rng: SmokeRng::new(1),
            phase: Phase::Calm,
            charge: 0.0,
            up: [0.0, 1.0, 0.0],
            reduced: false,
            time: 0.0,
        };
        s.set_up([0.0, 1000.0, 0.0]);
        s
    }

    #[test]
    fn particles_stay_on_the_cube_and_fall() {
        let mut s = smoke();
        s.burst(Face::PosY);
        for _ in 0..120 {
            s.step(1.0 / 60.0);
        }
        for q in &s.particles {
            let m = q.p.map(libm::fabsf).into_iter().fold(0.0, f32::max);
            assert!((m - 1.0).abs() < 1e-4, "on the surface: {:?}", q.p);
            assert_eq!(q.face, face_of(q.p));
            assert!(dot(q.v, normal(q.face)).abs() < 1e-4, "moves along its face");
        }
        // Two seconds in, most of the burst has rolled off the top face.
        let on_top = s.particles.iter().filter(|q| q.face == Face::PosY).count();
        assert!(
            on_top < s.particles.len() / 2,
            "{on_top} of {}",
            s.particles.len()
        );
    }

    #[test]
    fn a_shake_and_throw_fill_then_drain_the_cloud() {
        let mut s = smoke();
        s.shake_start();
        for _ in 0..60 {
            s.step(1.0 / 60.0);
        }
        let held = s.particles.iter().filter(|q| q.hold).count();
        // 1 s of a 1.6 s charge: 380 × 0.625 ≈ 238.
        assert!((230..=245).contains(&held), "{held}");
        assert!(
            s.particles.iter().all(|q| q.face != Face::NegY),
            "nothing spawns face down"
        );
        s.throw();
        assert_eq!(s.particles.len(), FULL);
        s.land();
        assert_eq!(s.particles.len(), FULL + 76 + EMBERS);
        assert!(s.has_smoke());
        for _ in 0..(5 * 60) {
            s.step(1.0 / 60.0);
        }
        assert!(!s.has_smoke(), "all drained within 5 s");
    }
}
