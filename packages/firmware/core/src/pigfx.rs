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

use core::f32::consts::PI;

use libm::{acosf, cosf, fabsf, powf, sinf, sqrtf};

use crate::display::PIXELS;

use crate::gfx::Painter;
use crate::pigs::Pose;

/// How long the pigs take to settle after the die lands.
pub const SETTLE_S: f32 = 0.9;

/// Canvas units per world unit.
const SCALE: f32 = 36.0;
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
    // World up in the pig's frame is the conjugate rotation of (0, 1, 0).
    let up = Quat {
        w: q.w,
        x: -q.x,
        y: -q.y,
        z: -q.z,
    }
    .rotate([0.0, 1.0, 0.0]);
    let (x, y, z) = (p.radii[0] * up[0], p.radii[1] * up[1], p.radii[2] * up[2]);
    sqrtf(x * x + y * y + z * z)
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
fn pose_rotation(pose: Pose, i: usize) -> Quat {
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
    let yaw = if i == 0 { 0.35 } else { -0.45 };
    Quat::axis_angle([0.0, 1.0, 0.0], yaw).mul(q)
}

fn ease_out(u: f32) -> f32 {
    let k = 1.0 - u;
    1.0 - k * k * k
}

/// A pig `u` (0–1) of the way from `from` to resting in `pose`. It rocks
/// down onto the table with a couple of shrinking bounces, and at `u` = 1 it
/// is exactly in the pose.
pub fn settle(i: usize, from: PigState, pose: Pose, u: f32) -> PigState {
    let u = u.clamp(0.0, 1.0);
    let e = ease_out(u);
    let q = from.q.slerp(pose_rotation(pose, i), e);
    let rest = [side(i) * 0.85, 0.0, 0.0];
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
pub fn rest(i: usize, pose: Pose) -> PigState {
    settle(i, tumble(i, 0.0), pose, 1.0)
}

/// What the faces show of the pigs.
#[derive(Clone, Copy, Debug)]
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
    },
}

/// Depth of each pixel drawn so far, for the face being drawn. Bigger is
/// nearer the viewer; 0 is nothing yet.
pub struct DepthBuffer([u16; PIXELS]);

impl DepthBuffer {
    pub const fn new() -> Self {
        Self([0; PIXELS])
    }
}

impl Default for DepthBuffer {
    fn default() -> Self {
        Self::new()
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

/// Draw the two pigs. `phase` shifts the tumble in time, so each face can
/// show a different moment of the same throw. `alpha` dims them. They are
/// drawn straight into the face's pixels, over whatever is there.
pub fn draw(p: &mut Painter, z: &mut DepthBuffer, scene: Scene, phase: f32, alpha: f32) {
    let states = match scene {
        Scene::Tumbling(t) => [tumble(0, t + phase), tumble(1, t + phase)],
        Scene::Settling { land, u, poses, .. } => [
            settle(0, tumble(0, land + phase), poses[0], u),
            settle(1, tumble(1, land + phase), poses[1], u),
        ],
    };
    let shrink = match scene {
        Scene::Settling { shrink, .. } => shrink,
        Scene::Tumbling(_) => 0.0,
    };
    let scale = SCALE * (1.0 - 0.45 * shrink);
    let lift = 66.0 * shrink;
    let (ce, se) = (cosf(ELEVATION), sinf(ELEVATION));
    // World to view: tip the table toward the viewer.
    let view: Mat = [[1.0, 0.0, 0.0], [0.0, ce, -se], [0.0, se, ce]];
    let xf = p.xf;
    let ppu = xf.px_per_unit();
    let to_px = ppu * scale;
    z.0.fill(0);

    // Every part of both pigs, far to near.
    let mut casts: [Option<Cast>; 2 * PARTS.len()] = core::array::from_fn(|_| None);
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
            casts[n] = Some(Cast {
                centre,
                r,
                radii: s,
                bx,
                by,
                bz,
                a: dot(bz, bz),
                tone: part.tone as f32,
                gloss: part.gloss,
            });
            n += 1;
        }
    }
    let casts = &mut casts[..n];
    for i in 1..casts.len() {
        let mut j = i;
        while j > 0 && depth_of(&casts[j - 1]) > depth_of(&casts[j]) {
            casts.swap(j - 1, j);
            j -= 1;
        }
    }

    let half = [0.5 * LIGHT[0], 0.5 * LIGHT[1], 0.5 * (LIGHT[2] + 1.0)];
    let hl = sqrtf(dot(half, half));
    let half = [half[0] / hl, half[1] / hl, half[2] / hl];
    let fb = p.framebuffer();
    for cast in casts.iter().flatten() {
        let rmax = cast.radii[0].max(cast.radii[1]).max(cast.radii[2]);
        // Canvas position of the centre, and its pixel bounds.
        let (cx, cy) = (
            CENTRE.0 + cast.centre[0] * scale,
            CENTRE.1 + 10.0 - lift - cast.centre[1] * scale,
        );
        let (px, py) = xf.forward(cx, cy);
        let reach = rmax * to_px + 1.5;
        let (x0, x1) = (
            libm::floorf(px - reach).max(0.0) as usize,
            (libm::ceilf(px + reach) as usize).min(96),
        );
        let (y0, y1) = (
            libm::floorf(py - reach).max(0.0) as usize,
            (libm::ceilf(py + reach) as usize).min(96),
        );
        // Pixels per unit of the ellipsoid's unit space, for edge softness.
        let rp = (cast.radii[0] + cast.radii[1] + cast.radii[2]) / 3.0 * to_px;
        for y in y0..y1 {
            for x in x0..x1 {
                let (ux, uy) = xf.inverse(x as f32 + 0.5, y as f32 + 0.5);
                let dx = (ux - CENTRE.0) / scale - cast.centre[0];
                let dy = -((uy - (CENTRE.1 + 10.0 - lift)) / scale) - cast.centre[1];
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
                let slot = y * 96 + x;
                if zq < z.0[slot] {
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
                let spec = powf(dot(nrm, half).max(0.0), 22.0) * cast.gloss;
                let level = cast.tone * (AMBIENT + (1.0 - AMBIENT) * diffuse) + 255.0 * spec * 0.7;
                // Never as bright as text, so the score still stands out.
                let lit = level.clamp(0.0, 235.0) * alpha;
                let under = fb.pixel(x, y) as f32;
                fb.set_pixel(x, y, (under + (lit - under) * cover).clamp(0.0, 255.0) as u8);
                if cover > 0.5 {
                    z.0[slot] = zq;
                }
            }
        }
    }
}

/// How near the viewer a part's centre is, for drawing far to near.
fn depth_of(c: &Option<Cast>) -> f32 {
    c.as_ref().map_or(0.0, |c| c.centre[2])
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
                let end = settle(i, start, pose, 1.0);
                let want = rest(i, pose);
                assert!(end.q.dot(want.q).abs() > 0.9999, "{pose:?}");
                assert!((end.pos[1] - want.pos[1]).abs() < 1e-4);
            }
        }
    }

    #[test]
    fn a_pig_rests_on_the_table_in_every_pose() {
        for pose in Pose::ALL {
            let low = lowest(&rest(0, pose));
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
            let st = rest(0, pose);
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
        use crate::display::Framebuffer;
        use crate::gfx::{Layer, Painter, Transform};
        let mut fb = Framebuffer::new();
        let mut layer = Layer::new();
        let mut z = DepthBuffer::new();
        let mut p = Painter::new(&mut fb, &mut layer, Transform::default());
        draw(&mut p, &mut z, Scene::Tumbling(0.7), 0.0, 1.0);
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
    fn slerp_ends_are_exact() {
        let a = Quat::axis_angle([0.0, 1.0, 0.0], 0.4);
        let b = Quat::axis_angle([1.0, 0.0, 0.0], 2.0);
        assert!(a.slerp(b, 0.0).dot(a) > 0.9999);
        assert!(a.slerp(b, 1.0).dot(b) > 0.9999);
    }
}
