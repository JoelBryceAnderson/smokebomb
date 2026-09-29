//! The pigs on the faces.
//!
//! Pig Toss has no smoke: the faces show two 3D pigs instead. They tumble
//! and bounce while the die is shaken and thrown, and once it lands they
//! settle into the poses the throw drew ([`crate::pigs::Pose`]). There is no
//! physics: the tumble is a fixed spin and bounce, and the landing blends
//! from wherever the tumble was to the pose, so a pig can only end up in the
//! pose it was dealt.
//!
//! A pig is a handful of spheres. They are rotated, projected, sorted far to
//! near and drawn as dark discs with a bright rim, so nearer parts hide what
//! is behind them. That gives a solid line-art pig with no triangle
//! rasteriser.

use core::f32::consts::PI;

use libm::{acosf, cosf, fabsf, sinf, sqrtf};

use crate::display::FG;
use crate::gfx::{Painter, Style};
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// Dark disc, bright rim.
    Body,
    /// A bright spot: an eye, or the dot on a pig's side.
    Spot,
}

/// One sphere of the pig: centre, radius and what it is. The pig faces +X,
/// stands on −Y, and its dot is on its −Z side.
struct Ball(f32, f32, f32, f32, Part);

const PIG: [Ball; 16] = [
    Ball(-0.30, 0.0, 0.0, 0.34, Part::Body),
    Ball(0.0, 0.0, 0.0, 0.38, Part::Body),
    Ball(0.30, 0.0, 0.0, 0.34, Part::Body),
    Ball(0.66, 0.08, 0.0, 0.27, Part::Body),
    Ball(0.92, 0.03, 0.0, 0.12, Part::Body),
    Ball(0.58, 0.33, 0.17, 0.10, Part::Body),
    Ball(0.58, 0.33, -0.17, 0.10, Part::Body),
    Ball(0.30, -0.38, 0.17, 0.09, Part::Body),
    Ball(0.30, -0.38, -0.17, 0.09, Part::Body),
    Ball(-0.30, -0.38, 0.17, 0.09, Part::Body),
    Ball(-0.30, -0.38, -0.17, 0.09, Part::Body),
    Ball(-0.68, 0.15, 0.0, 0.05, Part::Body),
    Ball(0.80, 0.17, 0.13, 0.04, Part::Spot),
    Ball(0.80, 0.17, -0.13, 0.04, Part::Spot),
    // The dot, on the −Z side.
    Ball(0.0, 0.03, -0.36, 0.09, Part::Spot),
    // Nostril.
    Ball(1.0, 0.05, 0.0, 0.03, Part::Spot),
];

/// Where a pig is and how it is turned. `y` is the height of the pig's
/// centre above the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PigState {
    q: Quat,
    pos: [f32; 3],
}

/// The lowest a pig turned by `q` reaches below its centre.
fn depth_below_centre(q: Quat) -> f32 {
    let mut low = 0.0f32;
    for Ball(x, y, z, r, part) in PIG {
        if part == Part::Body {
            low = low.min(q.rotate([x, y, z])[1] - r);
        }
    }
    -low
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
    Settling { land: f32, u: f32, poses: [Pose; 2] },
}

struct Disc {
    depth: f32,
    x: f32,
    y: f32,
    r: f32,
    part: Part,
}

/// Draw the two pigs. `phase` shifts the tumble in time, so each face can
/// show a different moment of the same throw. `alpha` dims them.
pub fn draw(p: &mut Painter, scene: Scene, phase: f32, alpha: f32) {
    let states = match scene {
        Scene::Tumbling(t) => [tumble(0, t + phase), tumble(1, t + phase)],
        Scene::Settling { land, u, poses } => [
            settle(0, tumble(0, land + phase), poses[0], u),
            settle(1, tumble(1, land + phase), poses[1], u),
        ],
    };
    let (ce, se) = (cosf(ELEVATION), sinf(ELEVATION));
    let mut discs: [Disc; 32] = core::array::from_fn(|_| Disc {
        depth: 0.0,
        x: 0.0,
        y: 0.0,
        r: 0.0,
        part: Part::Body,
    });
    let mut n = 0;
    for st in states {
        for Ball(bx, by, bz, r, part) in PIG {
            let w = st.q.rotate([bx, by, bz]);
            let (wx, wy, wz) = (w[0] + st.pos[0], w[1] + st.pos[1], w[2] + st.pos[2]);
            discs[n] = Disc {
                depth: wy * se + wz * ce,
                x: CENTRE.0 + wx * SCALE,
                y: CENTRE.1 + 10.0 - (wy * ce - wz * se) * SCALE,
                r: r * SCALE,
                part,
            };
            n += 1;
        }
    }
    // Far to near.
    let discs = &mut discs[..n];
    for i in 1..discs.len() {
        let mut j = i;
        while j > 0 && discs[j - 1].depth > discs[j].depth {
            discs.swap(j - 1, j);
            j -= 1;
        }
    }
    let spot = Style::new(FG, alpha, 0.0);
    for d in discs.iter() {
        match d.part {
            Part::Body => {
                // Nearer parts are brighter, so the pig reads as solid.
                let shade = (0x70 as f32 + d.depth * 0x30 as f32).clamp(0x38 as f32, 0xB0 as f32) as u8;
                p.fill_circle(d.x, d.y, d.r, Style::new(shade, alpha, 0.0));
                // The rim is a step brighter than the fill, so the joins
                // between a pig's parts show without ruling lines across it.
                let rim = Style::new(shade.saturating_add(0x48), alpha, 0.0);
                p.stroke_arc(d.x, d.y, d.r, 0.0, 2.0 * PI, 1.8, rim);
            }
            Part::Spot => p.fill_circle(d.x, d.y, d.r, spot),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let st = rest(0, pose);
            let mut low = f32::MAX;
            for Ball(x, y, z, r, part) in PIG {
                if part == Part::Body {
                    let w = st.q.rotate([x, y, z]);
                    low = low.min(w[1] + st.pos[1] - r);
                }
            }
            assert!(low.abs() < 1e-3, "{pose:?} floats or sinks by {low}");
        }
    }

    #[test]
    fn the_tumble_never_goes_through_the_table() {
        for i in 0..2 {
            let mut t = 0.0;
            while t < 6.0 {
                let st = tumble(i, t);
                let mut low = f32::MAX;
                for Ball(x, y, z, r, part) in PIG {
                    if part == Part::Body {
                        low = low.min(st.q.rotate([x, y, z])[1] + st.pos[1] - r);
                    }
                }
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
            st.q.rotate([0.0, 0.03, -0.36])[1]
        };
        assert!(up(Pose::SideDot) > 0.3, "dot side up");
        assert!(up(Pose::SidePlain) < -0.3, "plain side up");
    }

    #[test]
    fn slerp_ends_are_exact() {
        let a = Quat::axis_angle([0.0, 1.0, 0.0], 0.4);
        let b = Quat::axis_angle([1.0, 0.0, 0.0], 2.0);
        assert!(a.slerp(b, 0.0).dot(a) > 0.9999);
        assert!(a.slerp(b, 1.0).dot(b) > 0.9999);
    }
}
