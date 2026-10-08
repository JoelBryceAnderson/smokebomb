//! Which face is up, which way is north, and where each face's pixels sit on
//! the draped map.
//!
//! The die's faces and their drawing axes are the firmware's
//! ([`smokebomb_core::orientation::BASES`]): a face buffer is drawn in those
//! axes and the panel driver turns it to however the panel is mounted.
//!
//! The world is laid over the cube like a cloth over a box: the up face shows
//! the map around the player, and each side face shows the map beyond the
//! up face's edge on that side, folded down. Looking at any side face from
//! outside, its top row continues the up face across their shared edge.
//!
//! Directions here are signed die axes ([`Axis`]): north, east and up are
//! always three of the six face normals.

use smokebomb_core::orientation::BASES;
use smokebomb_hal::Face;

use crate::FACE;

/// A signed unit vector along a die axis.
pub type Axis = [i8; 3];

pub const fn neg(a: Axis) -> Axis {
    [-a[0], -a[1], -a[2]]
}

pub const fn dot(a: Axis, b: Axis) -> i32 {
    a[0] as i32 * b[0] as i32 + a[1] as i32 * b[1] as i32 + a[2] as i32 * b[2] as i32
}

pub const fn cross(a: Axis, b: Axis) -> Axis {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn axis_of(v: [f32; 3]) -> Axis {
    [v[0] as i8, v[1] as i8, v[2] as i8]
}

/// The face whose outward normal is `a`.
pub fn face_of(a: Axis) -> Face {
    Face::ALL
        .into_iter()
        .find(|f| axis_of(BASES[f.index()].n) == a)
        .expect("a unit axis")
}

pub fn normal(f: Face) -> Axis {
    axis_of(BASES[f.index()].n)
}

/// The die axis closest to `v` (any length).
pub fn nearest_axis(v: [f32; 3]) -> Axis {
    let a = [libm::fabsf(v[0]), libm::fabsf(v[1]), libm::fabsf(v[2])];
    let i = if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    };
    let mut out = [0i8; 3];
    out[i] = if v[i] < 0.0 { -1 } else { 1 };
    out
}

/// A compass direction on the map. North is up on the Game Boy screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compass {
    North,
    East,
    South,
    West,
}

impl Compass {
    pub const ALL: [Compass; 4] = [Compass::North, Compass::East, Compass::South, Compass::West];

    /// Screen-space unit step (x right, y down).
    pub const fn step(self) -> (i32, i32) {
        match self {
            Compass::North => (0, -1),
            Compass::East => (1, 0),
            Compass::South => (0, 1),
            Compass::West => (-1, 0),
        }
    }
}

/// Where the world is, in die axes: the sky, and map north.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heading {
    pub up: Axis,
    pub north: Axis,
}

impl Default for Heading {
    /// +Y up with north along −Z: the +Y face's own canvas-up, so the map
    /// starts upright in that face's drawing axes.
    fn default() -> Self {
        Heading {
            up: [0, 1, 0],
            north: [0, 0, -1],
        }
    }
}

impl Heading {
    /// Right-handed: east = north × up.
    pub const fn east(&self) -> Axis {
        cross(self.north, self.up)
    }

    pub const fn dir(&self, c: Compass) -> Axis {
        match c {
            Compass::North => self.north,
            Compass::South => neg(self.north),
            Compass::East => self.east(),
            Compass::West => neg(self.east()),
        }
    }

    pub fn up_face(&self) -> Face {
        face_of(self.up)
    }

    pub fn down_face(&self) -> Face {
        face_of(neg(self.up))
    }

    /// The side face on the `c` side.
    pub fn side_face(&self, c: Compass) -> Face {
        face_of(self.dir(c))
    }

    /// The die has rolled so that `new_up` points at the sky. The world
    /// didn't move, so its directions turn in die axes by the same quarter
    /// (or half) turn: rolling north over the north edge brings the south face
    /// up, and the old up face now points north.
    pub fn rolled_to(&self, new_up: Axis) -> Heading {
        if new_up == self.up {
            return *self;
        }
        if new_up == neg(self.up) {
            // Flipped over (no single roll does this): turn about east.
            return Heading {
                up: new_up,
                north: neg(self.north),
            };
        }
        // The quarter turn taking `up` to `new_up`: up → new_up,
        // new_up → −up, the axis perpendicular to both stays.
        let turn = |v: Axis| -> Axis {
            if v == self.up {
                new_up
            } else if v == neg(self.up) {
                neg(new_up)
            } else if v == new_up {
                neg(self.up)
            } else if v == neg(new_up) {
                self.up
            } else {
                v
            }
        };
        Heading {
            up: new_up,
            north: turn(self.north),
        }
    }
}

/// What a face shows in the current heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Top,
    Side(Compass),
    Bottom,
}

/// Maps a face's own upright coordinates to its buffer: index = `base +
/// du·u + dv·v` for `u, v` in `0..64`.
///
/// "Upright" is how a person sees the face: on a side face, `u` runs left to
/// right as seen from outside and `v` runs down from the top edge (the edge
/// it shares with the up face). On the up face, `u` runs east and `v` south
/// (map north at the top). On the bottom face, `u` east and `v` north.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FaceXf {
    pub base: i32,
    pub du: i32,
    pub dv: i32,
}

impl FaceXf {
    #[inline]
    pub fn index(&self, u: usize, v: usize) -> usize {
        (self.base + self.du * u as i32 + self.dv * v as i32) as usize
    }

    /// For `face`, with `u` growing along die direction `au` and `v` along
    /// `av`. The buffer's columns grow along the face's drawing x and its
    /// rows against its drawing y.
    fn new(face: Face, au: Axis, av: Axis) -> FaceXf {
        let b = &BASES[face.index()];
        let (x, down) = (axis_of(b.x), neg(axis_of(b.y)));
        let m = FACE as i32 - 1;
        let (cu, cv) = (dot(au, x), dot(av, x));
        let (ru, rv) = (dot(au, down), dot(av, down));
        let col0 = if cu + cv < 0 { m } else { 0 };
        let row0 = if ru + rv < 0 { m } else { 0 };
        FaceXf {
            base: row0 * FACE as i32 + col0,
            du: ru * FACE as i32 + cu,
            dv: rv * FACE as i32 + cv,
        }
    }
}

/// Every face's role and upright transform for one heading.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub heading: Heading,
    pub role: [Role; 6],
    pub xf: [FaceXf; 6],
}

impl Layout {
    pub fn new(h: Heading) -> Layout {
        let mut role = [Role::Bottom; 6];
        let mut xf = [FaceXf {
            base: 0,
            du: 0,
            dv: 0,
        }; 6];
        let east = h.east();
        for face in Face::ALL {
            let n = normal(face);
            let i = face.index();
            if n == h.up {
                role[i] = Role::Top;
                xf[i] = FaceXf::new(face, east, neg(h.north));
            } else if n == neg(h.up) {
                role[i] = Role::Bottom;
                xf[i] = FaceXf::new(face, east, h.north);
            } else {
                let c = Compass::ALL.into_iter().find(|&c| h.dir(c) == n).expect("a side");
                role[i] = Role::Side(c);
                // Seen from outside, right is up × outward normal.
                xf[i] = FaceXf::new(face, cross(h.up, n), neg(h.up));
            }
        }
        Layout { heading: h, role, xf }
    }

    pub fn face_with(&self, r: Role) -> Face {
        Face::ALL
            .into_iter()
            .find(|f| self.role[f.index()] == r)
            .expect("every role has a face")
    }
}

/// Where upright pixel (`u`, `v`) of a face with role `r` lands on the
/// draped map, relative to the centre of the up face (the up face covers
/// `-32..32` on both axes; x east, y south, in Game Boy pixels).
pub fn drape(r: Role, u: i32, v: i32) -> Option<(i32, i32)> {
    let h = FACE as i32 / 2;
    Some(match r {
        Role::Top => (u - h, v - h),
        Role::Side(Compass::South) => (u - h, h + v),
        Role::Side(Compass::North) => (h - 1 - u, -h - 1 - v),
        Role::Side(Compass::East) => (h + v, h - 1 - u),
        Role::Side(Compass::West) => (-h - 1 - v, u - h),
        Role::Bottom => return None,
    })
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;

    use super::*;

    /// Twice the 3D position of buffer pixel `i` of `face`'s centre, on a
    /// cube 128 units across (so every coordinate is an integer).
    fn point(face: Face, i: usize) -> [i32; 3] {
        let b = &BASES[face.index()];
        let (col, row) = ((i % FACE) as i32, (i / FACE) as i32);
        let s = 2 * col + 1 - FACE as i32;
        let t = FACE as i32 - (2 * row + 1);
        let (x, y, n) = (axis_of(b.x), axis_of(b.y), axis_of(b.n));
        [0, 1, 2].map(|k| n[k] as i32 * FACE as i32 + x[k] as i32 * s + y[k] as i32 * t)
    }

    fn headings() -> Vec<Heading> {
        // Every up axis with every perpendicular north.
        let axes: [Axis; 6] = [
            [1, 0, 0],
            [-1, 0, 0],
            [0, 1, 0],
            [0, -1, 0],
            [0, 0, 1],
            [0, 0, -1],
        ];
        let mut out = Vec::new();
        for up in axes {
            for north in axes {
                if dot(up, north) == 0 {
                    out.push(Heading { up, north });
                }
            }
        }
        out
    }

    #[test]
    fn transforms_are_permutations_of_the_buffer() {
        for h in headings() {
            let l = Layout::new(h);
            for f in Face::ALL {
                let mut seen = [false; FACE * FACE];
                for v in 0..FACE {
                    for u in 0..FACE {
                        let i = l.xf[f.index()].index(u, v);
                        assert!(!seen[i]);
                        seen[i] = true;
                    }
                }
            }
        }
    }

    #[test]
    fn upright_axes_point_the_right_way() {
        for h in headings() {
            let l = Layout::new(h);
            let east = h.east();
            for f in Face::ALL {
                let xf = l.xf[f.index()];
                let p0 = point(f, xf.index(0, 0));
                let pu = point(f, xf.index(1, 0));
                let pv = point(f, xf.index(0, 1));
                let du: [i32; 3] = [0, 1, 2].map(|k| (pu[k] - p0[k]) / 2);
                let dv: [i32; 3] = [0, 1, 2].map(|k| (pv[k] - p0[k]) / 2);
                let a = |v: Axis| [v[0] as i32, v[1] as i32, v[2] as i32];
                match l.role[f.index()] {
                    Role::Top => {
                        assert_eq!(du, a(east));
                        assert_eq!(dv, a(neg(h.north)));
                    }
                    Role::Side(_) => assert_eq!(dv, a(neg(h.up)), "v runs down"),
                    Role::Bottom => {}
                }
            }
        }
    }

    /// Wherever two pixels touch across the up face's edges, their draped map
    /// positions are neighbours: the map reads straight across the edge.
    #[test]
    fn the_drape_is_continuous_over_the_top_edges() {
        for h in headings() {
            let l = Layout::new(h);
            let top = l.face_with(Role::Top);
            let mut pairs = 0;
            for c in Compass::ALL {
                let side = l.face_with(Role::Side(c));
                let (txf, sxf) = (l.xf[top.index()], l.xf[side.index()]);
                for tv in 0..FACE {
                    for tu in 0..FACE {
                        let tp = point(top, txf.index(tu, tv));
                        for su in 0..FACE {
                            let sp = point(side, sxf.index(su, 0));
                            let d: i32 = (0..3).map(|k| (tp[k] - sp[k]).abs()).sum();
                            // Across the edge (1 unit up/down + 1 out) and not
                            // along it.
                            if d != 2 {
                                continue;
                            }
                            pairs += 1;
                            let a = drape(Role::Top, tu as i32, tv as i32).unwrap();
                            let b = drape(Role::Side(c), su as i32, 0).unwrap();
                            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                            assert_eq!((dx, dy), c.step(), "{h:?} {c:?} top ({tu},{tv}) side {su}");
                        }
                    }
                }
            }
            assert_eq!(pairs, 4 * FACE);
        }
    }

    #[test]
    fn rolling_keeps_the_world_still() {
        let h = Heading::default();
        // Roll north: the south face comes up and the old top points north.
        let south = h.dir(Compass::South);
        let r = h.rolled_to(south);
        assert_eq!(r.up, south);
        assert_eq!(r.north, h.up);
        assert_eq!(r.east(), h.east());
        // Roll east: the west face comes up, north stays.
        let r = h.rolled_to(h.dir(Compass::West));
        assert_eq!(r.north, h.north);
        assert_eq!(r.dir(Compass::East), h.up);
        // Four rolls the same way come back to the start.
        let mut r = h;
        for _ in 0..4 {
            r = r.rolled_to(r.dir(Compass::South));
        }
        assert_eq!(r, h);
    }
}
