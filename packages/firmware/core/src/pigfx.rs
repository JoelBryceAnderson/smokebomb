//! The pigs on the faces.
//!
//! Pig Toss has no smoke: the faces show two 3D pigs instead. They tumble
//! and bounce while the die is shaken and thrown, and once it lands they
//! settle into the poses the throw drew ([`crate::pigs::Pose`]). There is no
//! physics: the tumble is a fixed spin and bounce, and the landing blends
//! from wherever the tumble was to the pose, so a pig can only end up in the
//! pose it was dealt.
//!
//! A pig is a couple of dozen ellipsoids. Each is ray-cast per pixel against
//! a depth buffer and lit with one soft light, so the body is smooth, parts
//! hide each other exactly and the skin has a highlight. Eyes, hooves and the
//! dot on a pig's side are just dark ellipsoids.
//!
//! Every face shows the same pigs, so they are ray-cast once per frame into
//! a [`Canvas`] and then laid onto each face turned the way that face reads.
//! A scene that hasn't changed since the last frame (the pigs at rest under
//! the score) isn't cast again at all.

use core::f32::consts::PI;
use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use libm::{acosf, cosf, fabsf, sinf, sqrtf};

use smokebomb_hal::{Color, Pixel, Target};

use crate::display::{Framebuffer, PIXELS};

use crate::gfx::{Painter, Transform};
use crate::orientation::Quarter;
use crate::pigs::Pose;

/// How long the pigs take to settle after the die lands.
pub const SETTLE_S: f32 = 0.9;

/// Canvas units per world unit.
const SCALE: f32 = 33.0;
/// How far from the middle of the table each pig rests. Apart they leave a
/// clear gap, whatever pose they're in.
const APART_X: f32 = 1.3;
/// How much further apart apart-landed pigs draw once they shrink.
const SPREAD: f32 = 0.45;
/// How far touching pigs press into each other, world units.
const TOUCH_PRESS: f32 = 0.05;
/// Camera elevation above the table, radians.
const ELEVATION: f32 = 0.9;
/// Where the table's centre sits on the face.
const CENTRE: (f32, f32) = (0.0, -6.0);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Quat {
    w: f32,
    x: f32,
    y: f32,
    z: f32,
}

impl Quat {
    const IDENTITY: Quat = Quat {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    fn axis_angle(axis: [f32; 3], angle: f32) -> Quat {
        let (s, c) = (sinf(angle / 2.0), cosf(angle / 2.0));
        Quat {
            w: c,
            x: axis[0] * s,
            y: axis[1] * s,
            z: axis[2] * s,
        }
    }

    fn mul(self, o: Quat) -> Quat {
        Quat {
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
        }
    }

    fn dot(self, o: Quat) -> f32 {
        self.w * o.w + self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Along the short way round from `self` to `o`.
    fn slerp(self, mut o: Quat, t: f32) -> Quat {
        let mut d = self.dot(o);
        if d < 0.0 {
            o = Quat {
                w: -o.w,
                x: -o.x,
                y: -o.y,
                z: -o.z,
            };
            d = -d;
        }
        let (a, b) = if d > 0.9995 {
            (1.0 - t, t)
        } else {
            let theta = acosf(d.min(1.0));
            let s = sinf(theta);
            (sinf((1.0 - t) * theta) / s, sinf(t * theta) / s)
        };
        let q = Quat {
            w: self.w * a + o.w * b,
            x: self.x * a + o.x * b,
            y: self.y * a + o.y * b,
            z: self.z * a + o.z * b,
        };
        let n = sqrtf(q.dot(q));
        Quat {
            w: q.w / n,
            x: q.x / n,
            y: q.y / n,
            z: q.z / n,
        }
    }

    fn rotate(self, v: [f32; 3]) -> [f32; 3] {
        // v' = v + 2w(u×v) + 2u×(u×v), with u the vector part.
        let u = [self.x, self.y, self.z];
        let c1 = cross(u, v);
        let c2 = cross(u, c1);
        [
            v[0] + 2.0 * (self.w * c1[0] + c2[0]),
            v[1] + 2.0 * (self.w * c1[1] + c2[1]),
            v[2] + 2.0 * (self.w * c1[2] + c2[2]),
        ]
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// One piece of the pig: an axis-aligned ellipsoid in the pig's own frame.
/// The pig faces +X, stands on −Y, and has its dot on its −Z side.
struct Part {
    centre: [f32; 3],
    radii: [f32; 3],
    /// How bright the surface is under full light, 0–255. Dark parts are
    /// eyes, hooves, nostrils and the dot.
    tone: u8,
    /// How glossy it is, 0–1.
    gloss: f32,
    /// Whether the part bears the pig's weight (it can touch the table).
    solid: bool,
}

const fn part(centre: [f32; 3], radii: [f32; 3], tone: u8, gloss: f32, solid: bool) -> Part {
    Part {
        centre,
        radii,
        tone,
        gloss,
        solid,
    }
}

const PARTS: [Part; 20] = [
    // Barrel body, and the rump that makes it pear-shaped.
    part([0.05, 0.0, 0.0], [0.58, 0.44, 0.42], 215, 0.3, true),
    part([-0.32, -0.02, 0.0], [0.44, 0.43, 0.42], 215, 0.3, true),
    // Head, snout with nostrils, floppy ears.
    part([0.68, 0.02, 0.0], [0.34, 0.31, 0.30], 225, 0.3, true),
    part([1.02, -0.03, 0.0], [0.17, 0.14, 0.15], 245, 0.5, true),
    part([1.15, -0.01, 0.065], [0.035, 0.04, 0.035], 20, 0.6, false),
    part([1.15, -0.01, -0.065], [0.035, 0.04, 0.035], 20, 0.6, false),
    part([0.62, 0.34, 0.23], [0.13, 0.14, 0.07], 195, 0.15, true),
    part([0.62, 0.34, -0.23], [0.13, 0.14, 0.07], 195, 0.15, true),
    // Legs and dark hooves.
    part([0.36, -0.44, 0.23], [0.11, 0.16, 0.11], 200, 0.2, true),
    part([0.36, -0.44, -0.23], [0.11, 0.16, 0.11], 200, 0.2, true),
    part([-0.36, -0.44, 0.23], [0.11, 0.16, 0.11], 200, 0.2, true),
    part([-0.36, -0.44, -0.23], [0.11, 0.16, 0.11], 200, 0.2, true),
    part([0.36, -0.6, 0.23], [0.115, 0.06, 0.115], 60, 0.4, true),
    part([0.36, -0.6, -0.23], [0.115, 0.06, 0.115], 60, 0.4, true),
    // Tail.
    part([-0.76, 0.13, 0.0], [0.075, 0.075, 0.075], 205, 0.2, true),
    // Eyes.
    part([0.88, 0.13, 0.22], [0.05, 0.055, 0.04], 10, 0.9, false),
    part([0.88, 0.13, -0.22], [0.05, 0.055, 0.04], 10, 0.9, false),
    // The dot, a grey patch on the −Z side.
    part([0.0, 0.08, -0.405], [0.17, 0.14, 0.04], 80, 0.1, false),
    // Rear hooves.
    part([-0.36, -0.6, 0.23], [0.115, 0.06, 0.115], 60, 0.4, true),
    part([-0.36, -0.6, -0.23], [0.115, 0.06, 0.115], 60, 0.4, true),
];

/// Where a pig is and how it is turned. `y` is the height of the pig's
/// centre above the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PigState {
    q: Quat,
    pos: [f32; 3],
}

/// The lowest a pig turned by `q` reaches below its centre: each solid part
/// reaches `centre_y + radius along y` down, where an ellipsoid's radius
/// along a direction is the length of its radii scaled by that direction.
fn depth_below_centre(q: Quat) -> f32 {
    let mut low = 0.0f32;
    for p in PARTS.iter().filter(|p| p.solid) {
        low = low.min(q.rotate(p.centre)[1] - support(q, p));
    }
    -low
}

/// How far a part reaches from its centre along world up, when the pig is
/// turned by `q`.
fn support(q: Quat, p: &Part) -> f32 {
    support_along(q, p, [0.0, 1.0, 0.0])
}

/// How far a part reaches from its centre along the world direction `dir`
/// (a unit vector), when the pig is turned by `q`.
fn support_along(q: Quat, p: &Part, dir: [f32; 3]) -> f32 {
    // The direction in the pig's own frame is the conjugate rotation of it.
    let d = Quat {
        w: q.w,
        x: -q.x,
        y: -q.y,
        z: -q.z,
    }
    .rotate(dir);
    let (x, y, z) = (p.radii[0] * d[0], p.radii[1] * d[1], p.radii[2] * d[2]);
    sqrtf(x * x + y * y + z * z)
}

/// How far a pig turned by `q` reaches along world +X (`sign` 1) or −X
/// (`sign` −1) from its centre.
fn reach_x(q: Quat, sign: f32) -> f32 {
    PARTS
        .iter()
        .filter(|p| p.solid)
        .map(|p| sign * q.rotate(p.centre)[0] + support_along(q, p, [1.0, 0.0, 0.0]))
        .fold(f32::MIN, f32::max)
}

/// Which side of the table pig `i` lives on.
fn side(i: usize) -> f32 {
    if i == 0 {
        -1.0
    } else {
        1.0
    }
}

/// A pig in flight at `t` seconds: spinning about all three axes and
/// bouncing across its half of the table. Never below the table.
pub fn tumble(i: usize, t: f32) -> PigState {
    let s = side(i);
    let (a, b, c) = (3.1 + 0.6 * i as f32, 2.3 + 0.9 * i as f32, 1.7 + 0.4 * i as f32);
    let q = Quat::axis_angle([0.0, 0.0, 1.0], c * t * s)
        .mul(Quat::axis_angle([0.0, 1.0, 0.0], b * t))
        .mul(Quat::axis_angle([1.0, 0.0, 0.0], a * t));
    let bounce = fabsf(sinf(2.7 * t + 1.9 * i as f32));
    let x = s * (0.5 + 0.55 * sinf(1.3 * t + 2.0 * i as f32));
    let z = 0.45 * sinf(1.1 * t + 0.7 + i as f32);
    let y = depth_below_centre(q) + 0.15 + 1.4 * bounce;
    PigState { q, pos: [x, y, z] }
}

/// The pose's rotation of the standing pig, then turned a little about the
/// vertical so the two pigs don't line up.
fn pose_rotation(pose: Pose, i: usize, touching: bool) -> Quat {
    let x = [1.0, 0.0, 0.0];
    let z = [0.0, 0.0, 1.0];
    let q = match pose {
        Pose::Feet => Quat::IDENTITY,
        Pose::Back => Quat::axis_angle(x, PI),
        // A quarter turn about the pig's length puts the dot side up or down.
        Pose::SideDot => Quat::axis_angle(x, PI / 2.0),
        Pose::SidePlain => Quat::axis_angle(x, -PI / 2.0),
        // Nose down on the front feet.
        Pose::Nose => Quat::axis_angle(z, -1.15),
        // Leaning over on an ear as well.
        Pose::Ear => Quat::axis_angle(x, 0.55).mul(Quat::axis_angle(z, -0.95)),
    };
    // Apart, they sit a little crooked. Touching, they face each other.
    let yaw = match (touching, i) {
        (true, 0) => 0.0,
        (true, _) => PI,
        (false, 0) => 0.35,
        (false, _) => -0.45,
    };
    Quat::axis_angle([0.0, 1.0, 0.0], yaw).mul(q)
}

fn ease_out(u: f32) -> f32 {
    let k = 1.0 - u;
    1.0 - k * k * k
}

/// Where and how a pig ends up.
#[derive(Clone, Copy, Debug)]
pub struct Landing {
    pose: Pose,
    touching: bool,
    /// Where it rests along the table.
    x: f32,
}

/// Where the two pigs land. Normally they rest a clear gap apart, whatever
/// their poses. When they touch they face each other, close enough that
/// their nearest parts meet, however far each pose reaches.
pub fn landings(poses: [Pose; 2], touching: bool) -> [Landing; 2] {
    let x = if touching {
        let right = reach_x(pose_rotation(poses[0], 0, true), 1.0);
        let left = reach_x(pose_rotation(poses[1], 1, true), -1.0);
        // Overlap a hair, so they read as pressed together.
        let half = (right + left - TOUCH_PRESS) / 2.0;
        [-half, half]
    } else {
        [-APART_X, APART_X]
    };
    [0, 1].map(|i| Landing {
        pose: poses[i],
        touching,
        x: x[i],
    })
}

/// A pig `u` (0–1) of the way from `from` to resting as `to`. It rocks down
/// onto the table with a couple of shrinking bounces, and at `u` = 1 it is
/// exactly in the pose.
pub fn settle(i: usize, from: PigState, to: Landing, u: f32) -> PigState {
    let u = u.clamp(0.0, 1.0);
    let e = ease_out(u);
    let q = from.q.slerp(pose_rotation(to.pose, i, to.touching), e);
    let rest = [to.x, 0.0, 0.0];
    let bounce = if u >= 1.0 {
        0.0
    } else {
        let k = 1.0 - u;
        0.7 * k * k * fabsf(sinf(u * 3.0 * PI))
    };
    let floor = depth_below_centre(q);
    let y = from.pos[1] + (floor - from.pos[1]) * e + bounce;
    PigState {
        q,
        pos: [
            from.pos[0] + (rest[0] - from.pos[0]) * e,
            y.max(floor),
            from.pos[2] + (rest[2] - from.pos[2]) * e,
        ],
    }
}

/// Where a pig ends up.
pub fn rest(i: usize, poses: [Pose; 2], touching: bool) -> PigState {
    settle(i, tumble(i, 0.0), landings(poses, touching)[i], 1.0)
}

/// What the faces show of the pigs.
/// How a panel frames the pigs. The 96×96 die sees the table whole; a
/// smaller panel can zoom in, and pull the pigs' wandering in to match, so
/// that they stay big and on the face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Pig size against the 96×96 design.
    pub zoom: f32,
    /// How far across the table the pigs wander, tumble and rest, against
    /// the 96×96 design. Pigs that land touching still touch.
    pub spread: f32,
    /// The bounce height in the air, against the 96×96 design.
    pub hop: f32,
    /// Moves the table's centre down the face in the air, canvas units.
    pub drop: f32,
    /// Landed, the pair turns this far about the vertical (radians), as one
    /// piece so that touching pigs still touch. Then the view zooms to fit
    /// them, `fit` pixels across at most, and centres them. 0 keeps
    /// the 96×96 table's view.
    pub turn: f32,
    pub fit: f32,
    /// Lay a smaller panel's pixels 1:1 from the middle of the picture,
    /// rather than shrinking the whole picture onto it.
    pub crop: bool,
}

impl Lens {
    /// The 96×96 die's view.
    pub const WHOLE: Lens = Lens {
        zoom: 1.0,
        spread: 1.0,
        hop: 1.0,
        drop: 0.0,
        turn: 0.0,
        fit: 0.0,
        crop: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scene {
    /// In the air `t` seconds since the throw began.
    Tumbling(f32),
    /// Landed: at the moment of landing they were `land` seconds into the
    /// tumble, and `u` (0–1) of the way to resting in these poses.
    ///
    /// `shrink` (0–1) then moves them up and makes them smaller, to leave
    /// room for the score.
    Settling {
        land: f32,
        u: f32,
        shrink: f32,
        poses: [Pose; 2],
        /// Landed nose to nose, touching: the one bad case.
        touching: bool,
    },
}

/// Rows cast at a time. The depth buffer and the running shade only need to
/// cover a strip, not the face.
const STRIP: usize = 8;
/// Every part of both pigs.
const MAX_CASTS: usize = 2 * PARTS.len();

/// The pigs as every face shows them, cast once and shared. Held
/// unrotated; [`Canvas::lay_onto`] turns them for each face.
pub struct Canvas {
    /// The finished picture, a byte a pixel (see [`encode`]).
    picture: [u8; PIXELS],
    /// The parts being cast, far to near, and how many there are.
    casts: [Cast; MAX_CASTS],
    n: usize,
    /// For the strip being cast: the depth of each pixel so far (bigger is
    /// nearer; 0 is nothing yet), its shade already multiplied by how much
    /// the pigs cover it, and that coverage (both 0–255).
    z: [u16; 96 * STRIP],
    shade: [u8; 96 * STRIP],
    cover: [u8; 96 * STRIP],
    /// The pixels the picture holds anything in, unrotated.
    bounds: Region,
    /// The scene cast last, and through what, so an unchanged one isn't
    /// cast again.
    cast: Option<(Scene, Lens)>,
}

impl Canvas {
    /// An empty canvas, built in `slot`: at ~15 KB it's more than a small
    /// stack should hold and move.
    pub fn init(slot: &mut MaybeUninit<Self>) -> &mut Self {
        let p = slot.as_mut_ptr();
        // SAFETY: every field but `cast` is integers and floats, for which
        // zero is valid (and what an empty canvas holds); `cast` is written
        // after, before anything reads it.
        unsafe {
            p.write_bytes(0, 1);
            addr_of_mut!((*p).bounds).write(Region::EMPTY);
            addr_of_mut!((*p).cast).write(None);
            slot.assume_init_mut()
        }
    }

    /// An empty canvas, by value, for tests and the simulator.
    pub fn new() -> Self {
        let mut slot = MaybeUninit::uninit();
        Self::init(&mut slot);
        // SAFETY: `init` wrote it.
        unsafe { slot.assume_init() }
    }

    /// Cast the pigs for `scene`, unless that is what the canvas already
    /// holds. Returns the work it took, which is none for a repeat.
    pub fn draw(&mut self, scene: Scene) -> Stats {
        self.draw_through(scene, Lens::WHOLE)
    }

    /// [`Self::draw`], framed by `lens`.
    pub fn draw_through(&mut self, scene: Scene, lens: Lens) -> Stats {
        let scene = scene.at_rest_clamped();
        if self.cast == Some((scene, lens)) {
            return Stats::default();
        }
        self.cast = Some((scene, lens));
        let (states, view) = layout(scene, lens);
        self.n = prepare(&states, view, &mut self.casts);
        let mut stats = Stats::default();
        let mut lo = (usize::MAX, usize::MAX);
        let mut hi = (0, 0);
        for y0 in (0..96).step_by(STRIP) {
            let Self {
                picture,
                casts,
                n,
                z,
                shade,
                cover,
                ..
            } = self;
            shade.fill(0);
            cover.fill(0);
            let region = Region {
                x0: 0,
                y0,
                w: 96,
                h: STRIP,
            };
            let s = cast_block(
                Transform::default(),
                &casts[..*n],
                z,
                region,
                view,
                |x, y, lit, c| {
                    // Painted over what's under it: in premultiplied terms the
                    // shade and the coverage both go over what's there.
                    let i = (y - y0) * 96 + x;
                    let (s, a) = (shade[i] as f32, cover[i] as f32);
                    shade[i] = (s + (lit - s) * c + 0.5) as u8;
                    cover[i] = (a + (255.0 - a) * c + 0.5) as u8;
                },
            );
            stats.ray_tests += s.ray_tests;
            stats.shaded += s.shaded;
            for (i, (&s, &a)) in shade.iter().zip(cover.iter()).enumerate() {
                let (x, y) = (i % 96, y0 + i / 96);
                let b = encode(s, a);
                picture[y * 96 + x] = b;
                if b != 0 {
                    lo = (lo.0.min(x), lo.1.min(y));
                    hi = (hi.0.max(x + 1), hi.1.max(y + 1));
                }
            }
        }
        self.bounds = if hi.0 > lo.0 {
            Region {
                x0: lo.0,
                y0: lo.1,
                w: hi.0 - lo.0,
                h: hi.1 - lo.1,
            }
        } else {
            Region::EMPTY
        };
        stats
    }

    /// Lay the pigs onto a face turned by `rot`, over whatever it shows,
    /// dimmed by `alpha`. The picture is cast at 96×96; a smaller panel
    /// takes the middle of it 1:1 if the lens crops, else samples it whole,
    /// nearest.
    pub fn lay_onto<T: Target>(&self, fb: &mut Framebuffer<T>, rot: Quarter, alpha: f32) {
        self.lay_onto_tinted(fb, rot, alpha, Color::WHITE);
    }

    /// [`Self::lay_onto`], the pigs' shades taken as `tint`. White keeps them
    /// grey, exactly as before; a colour also fades their coverage with
    /// `alpha`, so they can fade out over the score (the 64×64 layout).
    pub fn lay_onto_tinted<T: Target>(&self, fb: &mut Framebuffer<T>, rot: Quarter, alpha: f32, tint: Color) {
        let put = |fb: &mut Framebuffer<T>, x: usize, y: usize, shade: f32, a: f32| {
            if tint == Color::WHITE {
                let under = fb.pixel(x, y).level() as f32;
                let v = under * (1.0 - a) + shade * a * alpha;
                fb.put(x, y, T::Pixel::grey((v + 0.5).clamp(0.0, 255.0) as u8));
            } else {
                // Here `alpha` fades the pigs out whole, coverage and all,
                // so a fading pig doesn't darken what's under it.
                fb.blend_rgb(x, y, tint.tint(shade), a * alpha);
            }
        };
        let b = self.bounds;
        if T::WIDTH == 96 {
            for sy in b.y0..b.y0 + b.h {
                for sx in b.x0..b.x0 + b.w {
                    let Some((shade, a)) = decode(self.picture[sy * 96 + sx]) else {
                        continue;
                    };
                    let (x, y) = turned(rot, sx, sy);
                    put(fb, x, y, shade, a);
                }
            }
            return;
        }
        let n = T::WIDTH;
        let crop = self.cast.is_some_and(|(_, l)| l.crop);
        let off = (96 - n) / 2;
        for ty in 0..n {
            for tx in 0..n {
                let (sx, sy) = if crop {
                    (tx + off, ty + off)
                } else {
                    ((tx * 96 + 48) / n, (ty * 96 + 48) / n)
                };
                let Some((shade, a)) = decode(self.picture[sy * 96 + sx]) else {
                    continue;
                };
                let (x, y) = rot.map_in(tx, ty, n);
                put(fb, x, y, shade, a);
            }
        }
    }
}

impl Default for Canvas {
    fn default() -> Self {
        Self::new()
    }
}

/// A pixel of the picture in a byte, from its premultiplied shade and its
/// coverage (0–255). Nearly every pixel the pigs cover, they cover fully:
/// those keep 7 bits of shade, within a level of what was cast. Edge pixels
/// keep 3 bits of coverage and 4 of shade, plenty for an edge. 0 is empty.
///
/// `1sssssss`: fully covered, shade 2·s.
/// `0cccssss`: covered c/8 (1–7), shade 17·s.
fn encode(shade: u8, cover: u8) -> u8 {
    if cover == 0 {
        return 0;
    }
    let a = cover as f32 / 255.0;
    // Undo the premultiply: the shade of what's there.
    let s = (shade as f32 / a).min(255.0);
    if cover >= 250 {
        return 0x80 | (libm::roundf(s / 2.0) as u8).min(127);
    }
    match (libm::roundf(a * 8.0) as u8).min(7) {
        0 => 0,
        c => c << 4 | (libm::roundf(s / 17.0) as u8).min(15),
    }
}

/// The shade (0–255) and coverage (0–1) of a picture byte, if any.
fn decode(b: u8) -> Option<(f32, f32)> {
    match b {
        0 => None,
        0x80.. => Some(((b & 0x7f) as f32 * 2.0, 1.0)),
        _ => Some(((b & 15) as f32 * 17.0, (b >> 4) as f32 / 8.0)),
    }
}

/// Where an unrotated pixel lands on a face turned by `rot`: what
/// [`Transform::quarter`] does to the canvas, in whole pixels.
fn turned(rot: Quarter, x: usize, y: usize) -> (usize, usize) {
    match rot {
        Quarter::R0 => (x, y),
        Quarter::R90 => (95 - y, x),
        Quarter::R180 => (95 - x, 95 - y),
        Quarter::R270 => (y, 95 - x),
    }
}

/// The unrotated pixel that lands at `(x, y)` on a face turned by `rot`.
#[cfg(test)]
fn unturned(rot: Quarter, x: usize, y: usize) -> (usize, usize) {
    match rot {
        Quarter::R0 => (x, y),
        Quarter::R90 => (y, 95 - x),
        Quarter::R180 => (95 - x, 95 - y),
        Quarter::R270 => (95 - y, x),
    }
}

impl Scene {
    /// The same picture, with the settle and shrink past their ends pinned
    /// at them, so that pigs at rest compare equal from frame to frame.
    fn at_rest_clamped(self) -> Scene {
        match self {
            Scene::Settling {
                land,
                u,
                shrink,
                poses,
                touching,
            } => Scene::Settling {
                land,
                u: u.clamp(0.0, 1.0),
                shrink: shrink.clamp(0.0, 1.0),
                poses,
                touching,
            },
            t => t,
        }
    }
}

/// The light, in view space (x right, y up, z toward the viewer): from the
/// upper left and in front.
const LIGHT: [f32; 3] = [-0.42, 0.62, 0.66];
const AMBIENT: f32 = 0.30;

/// Rows of a rotation as a 3×3 matrix.
type Mat = [[f32; 3]; 3];

fn matrix(q: Quat) -> Mat {
    let cols = [
        q.rotate([1.0, 0.0, 0.0]),
        q.rotate([0.0, 1.0, 0.0]),
        q.rotate([0.0, 0.0, 1.0]),
    ];
    // Column k of the matrix is the image of axis k.
    core::array::from_fn(|r| core::array::from_fn(|c| cols[c][r]))
}

fn mul(a: &Mat, b: &Mat) -> Mat {
    core::array::from_fn(|r| core::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

/// One ellipsoid, ready to ray-cast: everything that doesn't change from
/// pixel to pixel.
#[derive(Clone, Copy)]
struct Cast {
    /// View-space centre.
    centre: [f32; 3],
    /// The ellipsoid's axes in view space, rows of `R`, and its radii.
    r: Mat,
    radii: [f32; 3],
    /// Ray = A + z·B in the ellipsoid's unit space; A = dx·bx + dy·by.
    bx: [f32; 3],
    by: [f32; 3],
    bz: [f32; 3],
    a: f32,
    tone: f32,
    gloss: f32,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Where the two pigs are for `scene`, and how the table is seen.
fn layout(scene: Scene, lens: Lens) -> ([PigState; 2], View) {
    if lens != Lens::WHOLE {
        return framed(scene, lens);
    }
    let mut states = match scene {
        Scene::Tumbling(t) => [tumble(0, t), tumble(1, t)],
        Scene::Settling {
            land,
            u,
            poses,
            touching,
            ..
        } => {
            let to = landings(poses, touching);
            [
                settle(0, tumble(0, land), to[0], u),
                settle(1, tumble(1, land), to[1], u),
            ]
        }
    };
    let shrink = match scene {
        Scene::Settling { shrink, .. } => shrink,
        Scene::Tumbling(_) => 0.0,
    };
    // Small at the top of the face, pigs that landed apart draw further
    // apart, so that they plainly aren't touching. Touching ones stay put.
    if let Scene::Settling { touching: false, .. } = scene {
        for (i, st) in states.iter_mut().enumerate() {
            st.pos[0] += side(i) * SPREAD * shrink;
        }
    }
    let scale = SCALE * (1.0 - 0.45 * shrink);
    let lift = 66.0 * shrink;
    let view = View {
        elevation: ELEVATION,
        scale,
        origin: (CENTRE.0, CENTRE.1 + 10.0 - lift),
    };
    (states, view)
}

/// [`layout`] through a lens other than the whole table's. In the air the
/// pigs are zoomed in, with their wandering and bounce pulled in to match.
/// As they land, the view eases to the landed pair turned by `lens.turn`,
/// zoomed to fit and centred. The shrink is ignored: such a panel fades
/// the pigs instead.
fn framed(scene: Scene, lens: Lens) -> ([PigState; 2], View) {
    let squeeze = |mut st: PigState| {
        st.pos[0] *= lens.spread;
        st.pos[1] *= lens.hop;
        st.pos[2] *= lens.spread;
        st
    };
    let air = View {
        elevation: ELEVATION,
        scale: SCALE * lens.zoom,
        origin: (CENTRE.0, CENTRE.1 + 10.0 + lens.drop),
    };
    let Scene::Settling {
        land,
        u,
        poses,
        touching,
        ..
    } = scene
    else {
        let Scene::Tumbling(t) = scene else { unreachable!() };
        return ([squeeze(tumble(0, t)), squeeze(tumble(1, t))], air);
    };
    let to = landings(poses, touching);
    let states = [0, 1].map(|i| {
        let from = squeeze(tumble(i, land));
        // Where it rests, turned with the pair.
        // The pair turns in step with the settle.
        turned_about_y(settle(i, from, to[i], u), lens.turn * ease_out(u.clamp(0.0, 1.0)))
    });
    // The landed pair's bounds as the view sees them, and the view that
    // fits and centres them.
    let rest = [0, 1].map(|i| turned_about_y(settle(i, tumble(i, land), to[i], 1.0), lens.turn));
    let (lo, hi) = view_bounds(&rest);
    let span = (hi.0 - lo.0).max(hi.1 - lo.1);
    // `fit` is in pixels of the 96×96 picture.
    let fit_scale = (lens.fit / crate::gfx::K / span).min(SCALE * lens.zoom * 1.25);
    let fit = View {
        elevation: ELEVATION,
        scale: fit_scale,
        origin: (-0.5 * (lo.0 + hi.0) * fit_scale, 0.5 * (lo.1 + hi.1) * fit_scale),
    };
    let e = ease_out(u.clamp(0.0, 1.0));
    let mix = |a: f32, b: f32| a + (b - a) * e;
    let view = View {
        elevation: ELEVATION,
        scale: mix(air.scale, fit.scale),
        origin: (mix(air.origin.0, fit.origin.0), mix(air.origin.1, fit.origin.1)),
    };
    (states, view)
}

/// `st` turned `a` radians about the table's vertical through its centre.
fn turned_about_y(st: PigState, a: f32) -> PigState {
    let (c, sn) = (cosf(a), sinf(a));
    let (x, z) = (st.pos[0], st.pos[2]);
    PigState {
        q: Quat::axis_angle([0.0, 1.0, 0.0], a).mul(st.q),
        pos: [x * c + z * sn, st.pos[1], -x * sn + z * c],
    }
}

/// The smallest box, in world units across and up the view, that holds
/// the solid parts of the pigs in `states`: `((left, bottom), (right, top))`.
fn view_bounds(states: &[PigState]) -> ((f32, f32), (f32, f32)) {
    let (ce, se) = (cosf(ELEVATION), sinf(ELEVATION));
    let up = [0.0, ce, -se];
    let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
    for st in states {
        for p in PARTS.iter().filter(|p| p.solid) {
            let r = st.q.rotate(p.centre);
            let w = [r[0] + st.pos[0], r[1] + st.pos[1], r[2] + st.pos[2]];
            let (x, y) = (w[0], dot(w, up));
            let (rx, ry) = (
                support_along(st.q, p, [1.0, 0.0, 0.0]),
                support_along(st.q, p, up),
            );
            lo = (lo.0.min(x - rx), lo.1.min(y - ry));
            hi = (hi.0.max(x + rx), hi.1.max(y + ry));
        }
    }
    (lo, hi)
}

/// How much work a draw took, counted rather than timed, so a budget on it
/// means the same on every machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Rays cast: pixels tested against a part.
    pub ray_tests: u32,
    /// Of those, pixels that were lit and written.
    pub shaded: u32,
}

/// The block of the face a depth buffer covers.
#[derive(Clone, Copy)]
struct Region {
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
}

impl Region {
    const EMPTY: Region = Region {
        x0: 0,
        y0: 0,
        w: 0,
        h: 0,
    };
    #[cfg(test)]
    const FACE: Region = Region {
        x0: 0,
        y0: 0,
        w: 96,
        h: 96,
    };
}

/// How the table is seen: tipped up by `elevation`, `scale` canvas units
/// per world unit, with the table's centre at `origin` on the canvas.
#[derive(Clone, Copy)]
struct View {
    elevation: f32,
    scale: f32,
    origin: (f32, f32),
}

/// Every part of the pigs in `states`, ready to cast and sorted far to
/// near, into `out`. Returns how many.
fn prepare(states: &[PigState], v: View, out: &mut [Cast]) -> usize {
    let (ce, se) = (cosf(v.elevation), sinf(v.elevation));
    // World to view: tip the table toward the viewer.
    let view: Mat = [[1.0, 0.0, 0.0], [0.0, ce, -se], [0.0, se, ce]];
    let mut n = 0;
    for st in states {
        let r = mul(&view, &matrix(st.q));
        let pos = [
            view[0][0] * st.pos[0] + view[0][1] * st.pos[1] + view[0][2] * st.pos[2],
            view[1][0] * st.pos[0] + view[1][1] * st.pos[1] + view[1][2] * st.pos[2],
            view[2][0] * st.pos[0] + view[2][1] * st.pos[1] + view[2][2] * st.pos[2],
        ];
        for part in &PARTS {
            let c = part.centre;
            let centre = [
                r[0][0] * c[0] + r[0][1] * c[1] + r[0][2] * c[2] + pos[0],
                r[1][0] * c[0] + r[1][1] * c[1] + r[1][2] * c[2] + pos[1],
                r[2][0] * c[0] + r[2][1] * c[1] + r[2][2] * c[2] + pos[2],
            ];
            let s = part.radii;
            let row = |k: usize| [r[k][0] / s[0], r[k][1] / s[1], r[k][2] / s[2]];
            let (bx, by, bz) = (row(0), row(1), row(2));
            out[n] = Cast {
                centre,
                r,
                radii: s,
                bx,
                by,
                bz,
                a: dot(bz, bz),
                tone: part.tone as f32,
                gloss: part.gloss,
            };
            n += 1;
        }
    }
    let casts = &mut out[..n];
    for i in 1..casts.len() {
        let mut j = i;
        while j > 0 && casts[j - 1].centre[2] > casts[j].centre[2] {
            casts.swap(j - 1, j);
            j -= 1;
        }
    }
    n
}

/// Ray-cast prepared parts seen through `xf` into `region`, with `zbuf` as
/// its depth buffer. Each pixel hit goes to `plot` as its position, its
/// shade (0–255) and how much of it the part covers (0–1), nearer parts
/// after farther ones.
fn cast_block(
    xf: Transform,
    casts: &[Cast],
    zbuf: &mut [u16],
    region: Region,
    v: View,
    mut plot: impl FnMut(usize, usize, f32, f32),
) -> Stats {
    let mut stats = Stats::default();
    let View { scale, origin, .. } = v;
    let to_px = xf.px_per_unit() * scale;
    zbuf.fill(0);

    let half = [0.5 * LIGHT[0], 0.5 * LIGHT[1], 0.5 * (LIGHT[2] + 1.0)];
    let hl = sqrtf(dot(half, half));
    let half = [half[0] / hl, half[1] / hl, half[2] / hl];
    for cast in casts {
        let rmax = cast.radii[0].max(cast.radii[1]).max(cast.radii[2]);
        // Canvas position of the centre, and its pixel bounds.
        let (cx, cy) = (
            origin.0 + cast.centre[0] * scale,
            origin.1 - cast.centre[1] * scale,
        );
        let (px, py) = xf.forward(cx, cy);
        let reach = rmax * to_px + 1.5;
        let (x0, x1) = (
            (libm::floorf(px - reach).max(0.0) as usize).max(region.x0),
            (libm::ceilf(px + reach) as usize).min(region.x0 + region.w),
        );
        let (y0, y1) = (
            (libm::floorf(py - reach).max(0.0) as usize).max(region.y0),
            (libm::ceilf(py + reach) as usize).min(region.y0 + region.h),
        );
        // Off the block being drawn: nothing to cast.
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        // Pixels per unit of the ellipsoid's unit space, for edge softness.
        let rp = (cast.radii[0] + cast.radii[1] + cast.radii[2]) / 3.0 * to_px;
        for y in y0..y1 {
            for x in x0..x1 {
                stats.ray_tests += 1;
                let (ux, uy) = xf.inverse(x as f32 + 0.5, y as f32 + 0.5);
                let dx = (ux - origin.0) / scale - cast.centre[0];
                let dy = -((uy - origin.1) / scale) - cast.centre[1];
                let av = [
                    dx * cast.bx[0] + dy * cast.by[0],
                    dx * cast.bx[1] + dy * cast.by[1],
                    dx * cast.bx[2] + dy * cast.by[2],
                ];
                let b = dot(av, cast.bz);
                let c = dot(av, av) - 1.0;
                let disc = b * b - cast.a * c;
                // Distance of the ray from the ellipsoid's axis, in unit space.
                let miss = sqrtf((1.0 - disc / cast.a).max(0.0));
                let cover = ((1.0 - miss) * rp + 0.5).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let t = (-b + sqrtf(disc.max(0.0))) / cast.a;
                let depth = cast.centre[2] + t;
                let zq = ((depth + 4.0) * 8192.0).clamp(1.0, 65535.0) as u16;
                let slot = (y - region.y0) * region.w + (x - region.x0);
                if zq < zbuf[slot] {
                    continue;
                }
                // The surface normal, in view space.
                let u = [
                    av[0] + t * cast.bz[0],
                    av[1] + t * cast.bz[1],
                    av[2] + t * cast.bz[2],
                ];
                let v = [u[0] / cast.radii[0], u[1] / cast.radii[1], u[2] / cast.radii[2]];
                let mut nrm = [0.0f32; 3];
                for (k, n) in nrm.iter_mut().enumerate() {
                    *n = cast.r[k][0] * v[0] + cast.r[k][1] * v[1] + cast.r[k][2] * v[2];
                }
                let len = sqrtf(dot(nrm, nrm)).max(1e-6);
                let nrm = [nrm[0] / len, nrm[1] / len, nrm[2] / len];
                let diffuse = dot(nrm, LIGHT).max(0.0);
                // The highlight is (n·h)^22, by squaring: 16 + 4 + 2.
                let nh = dot(nrm, half).max(0.0);
                let (n2, n4) = (nh * nh, nh * nh * nh * nh);
                let n16 = n4 * n4 * n4 * n4;
                let spec = n16 * n4 * n2 * cast.gloss;
                let level = cast.tone * (AMBIENT + (1.0 - AMBIENT) * diffuse) + 255.0 * spec * 0.7;
                // Never as bright as text, so the score still stands out.
                plot(x, y, level.clamp(0.0, 235.0), cover);
                stats.shaded += 1;
                if cover > 0.5 {
                    zbuf[slot] = zq;
                }
            }
        }
    }
    stats
}

/// A single pig for a label: standing, turned three-quarters toward the
/// viewer, centred on `(cx, cy)` and about `r` canvas units across each way.
/// Cast a strip at a time with its own small buffers, so it can go on any
/// screen without much stack.
pub fn draw_icon<T: Target>(p: &mut Painter<T>, cx: f32, cy: f32, r: f32, alpha: f32) -> Stats {
    const SIDE: usize = 48;
    let (px, py) = p.xf.forward(cx, cy);
    let x0 = (libm::floorf(px) as i32 - SIDE as i32 / 2).clamp(0, (T::WIDTH - SIDE) as i32) as usize;
    let top = (libm::floorf(py) as i32 - SIDE as i32 / 2).clamp(0, (T::HEIGHT - SIDE) as i32) as usize;
    let pig = PigState {
        q: Quat::axis_angle([0.0, 1.0, 0.0], -0.95),
        pos: [0.0, 0.0, 0.0],
    };
    let view = View {
        elevation: 0.32,
        scale: r / 1.05,
        origin: (cx + 0.06 * r, cy + 0.06 * r),
    };
    let mut casts = [Cast::NONE; PARTS.len()];
    let n = prepare(&[pig], view, &mut casts);
    let mut zbuf = [0u16; SIDE * STRIP];
    let xf = p.xf;
    let fb = p.framebuffer();
    let mut stats = Stats::default();
    for y0 in (top..top + SIDE).step_by(STRIP) {
        let region = Region {
            x0,
            y0,
            w: SIDE,
            h: STRIP,
        };
        let s = cast_block(xf, &casts[..n], &mut zbuf, region, view, |x, y, lit, cover| {
            let under = fb.pixel(x, y).level() as f32;
            fb.put(
                x,
                y,
                T::Pixel::grey((under + (lit * alpha - under) * cover).clamp(0.0, 255.0) as u8),
            );
        });
        stats.ray_tests += s.ray_tests;
        stats.shaded += s.shaded;
    }
    stats
}

impl Cast {
    const NONE: Cast = Cast {
        centre: [0.0; 3],
        r: [[0.0; 3]; 3],
        radii: [0.0; 3],
        bx: [0.0; 3],
        by: [0.0; 3],
        bz: [0.0; 3],
        a: 0.0,
        tone: 0.0,
        gloss: 0.0,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The height of the pig's lowest point above the table.
    fn lowest(st: &PigState) -> f32 {
        PARTS
            .iter()
            .filter(|p| p.solid)
            .map(|p| st.q.rotate(p.centre)[1] + st.pos[1] - support(st.q, p))
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn a_pig_lands_exactly_in_its_pose() {
        for pose in Pose::ALL {
            for i in 0..2 {
                let start = tumble(i, 1.234);
                let poses = [pose, Pose::Feet];
                let end = settle(i, start, landings(poses, false)[i], 1.0);
                let want = rest(i, poses, false);
                assert!(end.q.dot(want.q).abs() > 0.9999, "{pose:?}");
                assert!((end.pos[1] - want.pos[1]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn a_pig_rests_on_the_table_in_every_pose() {
        for pose in Pose::ALL {
            let low = lowest(&rest(0, [pose, pose], false));
            assert!(low.abs() < 1e-3, "{pose:?} floats or sinks by {low}");
        }
    }

    #[test]
    fn the_tumble_never_goes_through_the_table() {
        for i in 0..2 {
            let mut t = 0.0;
            while t < 6.0 {
                let low = lowest(&tumble(i, t));
                assert!(low > -1e-3, "pig {i} at {t}s is {low} under the table");
                t += 0.05;
            }
        }
    }

    #[test]
    fn the_dot_side_is_up_for_a_dot_side_pose() {
        let up = |pose| {
            let st = rest(0, [pose, pose], false);
            // The dot's centre, turned into the world.
            st.q.rotate(PARTS[17].centre)[1]
        };
        assert!(up(Pose::SideDot) > 0.3, "dot side up");
        assert!(up(Pose::SidePlain) < -0.3, "plain side up");
    }

    #[test]
    fn the_dot_is_where_the_model_says() {
        assert_eq!(PARTS[17].tone, 80, "PARTS[17] is the dot");
        assert!(PARTS[17].centre[2] < 0.0, "on the −Z side");
    }

    #[test]
    fn drawing_puts_pigs_on_the_face_and_leaves_the_rest_alone() {
        let mut fb = Framebuffer::<smokebomb_hal::Grey96>::new();
        let mut canvas = Canvas::new();
        canvas.draw(Scene::Tumbling(0.7));
        canvas.lay_onto(&mut fb, Quarter::R0, 1.0);
        let lit = fb.pixels().iter().filter(|&&v| v > 40).count();
        assert!(lit > 400, "two pigs are a decent number of pixels: {lit}");
        assert!(
            fb.pixels().iter().all(|&v| v < 250),
            "no clipped whites: 255 is for text"
        );
        // The corners stay dark.
        assert_eq!(fb.pixel(0, 0), 0);
        assert_eq!(fb.pixel(95, 95), 0);
    }

    #[test]
    fn the_icon_is_a_pig_inside_its_box() {
        use crate::gfx::Layer;
        let mut fb = Framebuffer::<smokebomb_hal::Grey96>::new();
        let mut layer = Layer::new();
        let mut p = Painter::new(&mut fb, &mut layer, Transform::default());
        draw_icon(&mut p, 0.0, -21.0, 36.0, 0.85);
        let lit = fb.pixels().iter().filter(|&&v| v > 40).count();
        assert!(lit > 300, "a pig, not a speck: {lit}");
        // Nothing beyond the icon's box: the label's text lives below it.
        for y in 60..96 {
            for x in 0..96 {
                assert_eq!(fb.pixel(x, y), 0, "({x}, {y})");
            }
        }
    }

    #[test]
    fn normal_pigs_always_land_clear_of_each_other() {
        for a in Pose::ALL {
            for b in Pose::ALL {
                let [l, r] = [rest(0, [a, b], false), rest(1, [a, b], false)];
                let gap = (r.pos[0] - reach_x(r.q, -1.0)) - (l.pos[0] + reach_x(l.q, 1.0));
                assert!(gap > 0.25, "{a:?} and {b:?} come within {gap}");
            }
        }
    }

    #[test]
    fn touching_pigs_really_touch_in_every_pose() {
        for a in Pose::ALL {
            for b in Pose::ALL {
                let [l, r] = [rest(0, [a, b], true), rest(1, [a, b], true)];
                let gap = (r.pos[0] - reach_x(r.q, -1.0)) - (l.pos[0] + reach_x(l.q, 1.0));
                assert!(gap < 0.0 && gap > -0.1, "{a:?} and {b:?}: {gap}");
            }
        }
    }

    /// The most work a frame's pigs take, over tumbles, every settled pose
    /// pair (small and large) and the icon.
    fn worst_case_work() -> (Stats, Stats) {
        use crate::gfx::Layer;
        let mut fb = Framebuffer::<smokebomb_hal::Grey96>::new();
        let mut layer = Layer::new();
        let mut canvas = Canvas::new();
        let mut face = Stats::default();
        let note = |s: Stats, into: &mut Stats| {
            into.ray_tests = into.ray_tests.max(s.ray_tests);
            into.shaded = into.shaded.max(s.shaded);
        };
        let mut scenes = std::vec::Vec::new();
        for k in 0..240 {
            scenes.push(Scene::Tumbling(k as f32 * 0.025));
        }
        for a in Pose::ALL {
            for b in Pose::ALL {
                for touching in [false, true] {
                    for shrink in [0.0, 1.0] {
                        scenes.push(Scene::Settling {
                            land: 1.0,
                            u: 1.0,
                            shrink,
                            poses: [a, b],
                            touching,
                        });
                    }
                }
            }
        }
        for scene in scenes {
            note(canvas.draw(scene), &mut face);
        }
        let mut p = Painter::new(&mut fb, &mut layer, Transform::default());
        let icon = draw_icon(&mut p, 0.0, -21.0, 36.0, 0.85);
        (face, icon)
    }

    /// The work a frame's pigs may take, in rays cast: they are cast once
    /// and shared by every face. Counted, not timed, so
    /// it means the same on every machine. Measured worst case is about
    /// 5,800 rays (2,000 lit pixels) for a face and 3,000 for the icon, over
    /// every tumble, pose pair, size and spacing; the budget is a quarter
    /// above that. If a change to the model or renderer trips this, the
    /// change made the pigs more expensive on the die: check that on
    /// hardware before raising it.
    const FACE_RAY_BUDGET: u32 = 7_250;
    const FACE_SHADED_BUDGET: u32 = 2_450;
    const ICON_RAY_BUDGET: u32 = 3_750;

    #[test]
    fn the_pigs_stay_within_their_work_budget() {
        let (face, icon) = worst_case_work();
        assert!(
            face.ray_tests <= FACE_RAY_BUDGET,
            "a frame casts {} rays, over the budget of {FACE_RAY_BUDGET}",
            face.ray_tests
        );
        assert!(
            face.shaded <= FACE_SHADED_BUDGET,
            "a frame lights {} pixels, over the budget of {FACE_SHADED_BUDGET}",
            face.shaded
        );
        assert!(
            icon.ray_tests <= ICON_RAY_BUDGET,
            "the icon casts {} rays, over the budget of {ICON_RAY_BUDGET}",
            icon.ray_tests
        );
    }

    /// Wall-clock cost of a frame's pigs, cast once and laid onto all six
    /// faces, for a person to run in release:
    /// `cargo test --release -p smokebomb-core --lib time_a_frame -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn time_a_frame() {
        let mut faces = [Framebuffer::<smokebomb_hal::Grey96>::new(); 6];
        let mut canvas = Canvas::new();
        let n = 2000;
        let start = std::time::Instant::now();
        let mut rays = 0;
        for i in 0..n {
            rays += canvas.draw(Scene::Tumbling(i as f32 * 0.016)).ray_tests;
            for fb in &mut faces {
                canvas.lay_onto(fb, Quarter::R90, 1.0);
            }
        }
        let each = start.elapsed() / n;
        std::println!("{each:?} per frame, {} rays on average", rays / n);
    }

    #[test]
    fn every_face_shows_the_pigs_it_would_have_cast_itself() {
        use crate::gfx::Layer;
        let scene = Scene::Tumbling(0.7);
        let mut canvas = Canvas::new();
        canvas.draw(scene);
        for rot in [Quarter::R0, Quarter::R90, Quarter::R180, Quarter::R270] {
            let mut shared = Framebuffer::<smokebomb_hal::Grey96>::new();
            canvas.lay_onto(&mut shared, rot, 1.0);
            // Cast straight onto a face turned by `rot`, a whole face at
            // once, at full precision.
            let mut own = Framebuffer::<smokebomb_hal::Grey96>::new();
            let mut layer = Layer::new();
            let mut p = Painter::new(&mut own, &mut layer, Transform::quarter(rot));
            let xf = p.xf;
            let fb = p.framebuffer();
            let (states, view) = layout(scene, Lens::WHOLE);
            let mut casts = [Cast::NONE; MAX_CASTS];
            let n = prepare(&states, view, &mut casts);
            let mut z = [0u16; PIXELS];
            cast_block(xf, &casts[..n], &mut z, Region::FACE, view, |x, y, lit, c| {
                let under = fb.pixel(x, y) as f32;
                fb.set_pixel(x, y, (under + (lit - under) * c) as u8);
            });
            // Where the pigs cover a pixel fully, within a quarter of a grey
            // step of the panel (the reference rounds each blend down, the
            // canvas to nearest); the packed edges within a step (17 levels).
            for (i, (&a, &b)) in shared.pixels().iter().zip(own.pixels()).enumerate() {
                let (sx, sy) = unturned(rot, i % 96, i / 96);
                let solid = canvas.picture[sy * 96 + sx] >= 0x80;
                let limit = if solid { 4 } else { 20 };
                assert!(a.abs_diff(b) <= limit, "{rot:?} pixel {i}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn a_picture_byte_keeps_the_shade_and_the_edge() {
        assert_eq!(decode(encode(0, 0)), None);
        // Fully covered: within a level.
        for shade in [0u8, 1, 100, 201, 235] {
            let (s, a) = decode(encode(shade, 255)).unwrap();
            assert_eq!(a, 1.0);
            assert!((s - shade as f32).abs() <= 1.0, "{shade} came back {s}");
        }
        // Half covered: the premultiplied shade comes back near enough.
        let (s, a) = decode(encode(60, 128)).unwrap();
        assert!((s * a - 60.0).abs() < 10.0, "{s} × {a}");
        // Covered so little it rounds away.
        assert_eq!(decode(encode(2, 10)), None);
    }

    #[test]
    fn the_canvas_is_small() {
        let size = core::mem::size_of::<Canvas>();
        // The picture (9 KB), the parts being cast (4 KB) and one strip's
        // buffers (3 KB).
        assert!(size <= 17 * 1024, "the canvas is {size} bytes");
    }

    #[test]
    fn pigs_at_rest_are_cast_once() {
        let at = |u: f32, shrink: f32| Scene::Settling {
            land: 1.0,
            u,
            shrink,
            poses: [Pose::Feet, Pose::Back],
            touching: false,
        };
        let mut canvas = Canvas::new();
        assert!(canvas.draw(at(0.5, 0.0)).ray_tests > 0);
        assert!(canvas.draw(at(1.0, 1.0)).ray_tests > 0);
        assert_eq!(
            canvas.draw(at(1.4, 1.0)).ray_tests,
            0,
            "past the settle, nothing moves"
        );
        assert_eq!(canvas.draw(at(3.0, 1.0)).ray_tests, 0);
        assert!(canvas.draw(Scene::Tumbling(0.2)).ray_tests > 0);
    }

    #[test]
    fn slerp_ends_are_exact() {
        let a = Quat::axis_angle([0.0, 1.0, 0.0], 0.4);
        let b = Quat::axis_angle([1.0, 0.0, 0.0], 2.0);
        assert!(a.slerp(b, 0.0).dot(a) > 0.9999);
        assert!(a.slerp(b, 1.0).dot(b) > 0.9999);
    }
}

#[cfg(test)]
mod lens_tests {
    use super::*;

    /// How much of the pigs a 64×64 face crops away (the share of the
    /// picture's coverage outside the middle 64×64), worst over a long
    /// tumble and every landing, and how big the landed pigs are.
    #[test]
    fn a_64_face_keeps_the_pigs_on_it() {
        use crate::target::DisplayTarget;
        let lens = <smokebomb_hal::Rgb64 as DisplayTarget>::pig_lens();
        let mut canvas = Canvas::new();
        let mut off = |scene: Scene| {
            canvas.draw_through(scene, lens);
            let (mut all, mut out) = (0u32, 0u32);
            for y in 0..96 {
                for x in 0..96 {
                    if let Some((_, a)) = decode(canvas.picture[y * 96 + x]) {
                        let c = (a * 255.0) as u32;
                        all += c;
                        if !(16..80).contains(&x) || !(16..80).contains(&y) {
                            out += c;
                        }
                    }
                }
            }
            let b = canvas.bounds;
            (out as f32 / all.max(1) as f32, b.w.max(b.h))
        };
        let mut worst_air: f32 = 0.0;
        for k in 0..400 {
            worst_air = worst_air.max(off(Scene::Tumbling(k as f32 * 0.025)).0);
        }
        let (mut worst_rest, mut smallest): (f32, usize) = (0.0, 96);
        for pa in Pose::ALL {
            for pb in Pose::ALL {
                for touching in [false, true] {
                    for land in [0.3, 1.1, 2.6] {
                        let (o, span) = off(Scene::Settling {
                            land,
                            u: 1.0,
                            shrink: 0.0,
                            poses: [pa, pb],
                            touching,
                        });
                        worst_rest = worst_rest.max(o);
                        smallest = smallest.min(span);
                    }
                }
            }
        }
        assert!(worst_rest < 0.01, "landed pigs {worst_rest} off the face");
        assert!(worst_air < 0.12, "tumbling pigs {worst_air} off the face");
        // Landed, the pair fills most of the face (the 96×96 table's pigs
        // shrunk onto it spanned about 30 px).
        assert!(smallest >= 50, "landed pigs only {smallest} px across");
    }
}
