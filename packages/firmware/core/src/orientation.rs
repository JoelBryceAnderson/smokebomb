//! Which way is up on each face (SIM_SPEC B2).
//!
//! Side faces keep their text upright relative to gravity, snapped to 90°
//! steps with hysteresis so it doesn't flicker. The top and bottom faces
//! can't know where the viewer is, so they keep whatever orientation they
//! last had.

use core::f32::consts::{FRAC_PI_2, FRAC_PI_4};

use smokebomb_hal::{Face, ImuSample, FACE_COUNT, PANEL_WIDTH};

/// A face's drawing axes in die coordinates, matching the mockup's face
/// meshes: `x` is canvas-right, `y` is canvas-up (canvas y grows downward),
/// `n` is the outward normal.
#[derive(Clone, Copy, Debug)]
pub struct FaceBasis {
    pub x: [f32; 3],
    pub y: [f32; 3],
    pub n: [f32; 3],
}

pub const BASES: [FaceBasis; FACE_COUNT] = [
    FaceBasis {
        x: [0.0, 0.0, -1.0],
        y: [0.0, 1.0, 0.0],
        n: [1.0, 0.0, 0.0],
    }, // +X
    FaceBasis {
        x: [0.0, 0.0, 1.0],
        y: [0.0, 1.0, 0.0],
        n: [-1.0, 0.0, 0.0],
    }, // −X
    FaceBasis {
        x: [1.0, 0.0, 0.0],
        y: [0.0, 0.0, -1.0],
        n: [0.0, 1.0, 0.0],
    }, // +Y
    FaceBasis {
        x: [1.0, 0.0, 0.0],
        y: [0.0, 0.0, 1.0],
        n: [0.0, -1.0, 0.0],
    }, // −Y
    FaceBasis {
        x: [1.0, 0.0, 0.0],
        y: [0.0, 1.0, 0.0],
        n: [0.0, 0.0, 1.0],
    }, // +Z
    FaceBasis {
        x: [-1.0, 0.0, 0.0],
        y: [0.0, 1.0, 0.0],
        n: [0.0, 0.0, -1.0],
    }, // −Z
];

/// Content rotation in quarter turns, clockwise on the panel (the canvas
/// `rotate(k·π/2)` of the mockup).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quarter {
    #[default]
    R0,
    R90,
    R180,
    R270,
}

impl Quarter {
    pub fn from_angle(radians: f32) -> Quarter {
        match (libm::roundf(radians / FRAC_PI_2) as i32).rem_euclid(4) {
            0 => Quarter::R0,
            1 => Quarter::R90,
            2 => Quarter::R180,
            _ => Quarter::R270,
        }
    }

    /// Where content pixel `(x, y)` lands on the panel.
    pub fn map(self, x: usize, y: usize) -> (usize, usize) {
        let m = PANEL_WIDTH - 1;
        match self {
            Quarter::R0 => (x, y),
            Quarter::R90 => (m - y, x),
            Quarter::R180 => (m - x, m - y),
            Quarter::R270 => (y, m - x),
        }
    }
}

/// Snapped text angle per face, with the mockup's hysteresis.
#[derive(Clone, Debug, Default)]
pub struct TextOrientation {
    stable: [Option<f32>; FACE_COUNT],
}

/// Raw angle must drift this far from the current one before it changes.
const HYSTERESIS: f32 = FRAC_PI_4 + 0.3;

impl TextOrientation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn quarter(&self, face: Face) -> Quarter {
        Quarter::from_angle(self.stable[face.index()].unwrap_or(0.0))
    }

    /// Update every face from `up`, the gravity-up direction in die
    /// coordinates (any length).
    pub fn update(&mut self, up: [f32; 3]) {
        let up_axis = dominant_axis(up);
        for face in Face::ALL {
            let b = &BASES[face.index()];
            let slot = &mut self.stable[face.index()];
            if dominant_axis(b.n) == up_axis {
                // Top and bottom: locked.
                slot.get_or_insert(0.0);
                continue;
            }
            let Some(raw) = upright_angle(b, up) else {
                continue; // no clear direction: keep what's there
            };
            let snapped = libm::roundf(raw / FRAC_PI_2) * FRAC_PI_2;
            let current = *slot.get_or_insert(snapped);
            let diff = libm::atan2f(libm::sinf(raw - current), libm::cosf(raw - current));
            if libm::fabsf(diff) > HYSTERESIS {
                *slot = Some(snapped);
            }
        }
    }
}

/// The text angle that stands upright on a side face with the sky along
/// `up`: text "down" is the opposite of the sky, projected onto the face.
fn upright_angle(b: &FaceBasis, up: [f32; 3]) -> Option<f32> {
    let k = dot(up, b.n);
    let down = [
        -(up[0] - b.n[0] * k),
        -(up[1] - b.n[1] * k),
        -(up[2] - b.n[2] * k),
    ];
    if dot(down, down) < 1e-4 * dot(up, up) {
        return None;
    }
    let (lx, ly) = (dot(down, b.x), dot(down, b.y));
    Some(libm::atan2f(-lx, -ly))
}

/// Upright text for `face` if the sky were along `up` (any length), or
/// `None` when `face` would be the top or bottom face.
pub fn upright(face: Face, up: [f32; 3]) -> Option<Quarter> {
    let b = &BASES[face.index()];
    if dominant_axis(b.n) == dominant_axis(up) {
        return None;
    }
    upright_angle(b, up).map(Quarter::from_angle)
}

/// Gravity-up in die coordinates from an accelerometer sample: at rest the
/// sensor reads +1 g along the axis pointing at the sky.
pub fn up_from(sample: &ImuSample) -> [f32; 3] {
    sample.accel_mg.map(|a| a as f32)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn dominant_axis(v: [f32; 3]) -> usize {
    let a = v.map(libm::fabsf);
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upright_die_keeps_side_text_upright() {
        let mut t = TextOrientation::new();
        t.update([0.0, 1.0, 0.0]); // +Y up: every side face's canvas-up is +Y
        for f in [Face::PosX, Face::NegX, Face::PosZ, Face::NegZ] {
            assert_eq!(t.quarter(f), Quarter::R0, "{f:?}");
        }
    }

    #[test]
    fn lying_on_a_side_turns_the_text() {
        let mut t = TextOrientation::new();
        // +X up: on the +Z face the sky is canvas-right, so the text turns a
        // quarter clockwise to point its top at it.
        t.update([1.0, 0.0, 0.0]);
        assert_eq!(t.quarter(Face::PosZ), Quarter::R90);
        // Upside down (−Y up): side faces rotate half a turn.
        let mut t = TextOrientation::new();
        t.update([0.0, -1.0, 0.0]);
        assert_eq!(t.quarter(Face::PosZ), Quarter::R180);
    }

    #[test]
    fn hysteresis_ignores_small_tilts() {
        let mut t = TextOrientation::new();
        t.update([0.0, 1.0, 0.0]);
        // Tilt 60° towards +X: past 45° but inside 45° + 0.3 rad.
        let a = 60f32.to_radians();
        t.update([libm::sinf(a), libm::cosf(a), 0.0]);
        assert_eq!(t.quarter(Face::PosZ), Quarter::R0);
        let a = 80f32.to_radians();
        t.update([libm::sinf(a), libm::cosf(a), 0.0]);
        assert_eq!(t.quarter(Face::PosZ), Quarter::R90);
    }

    #[test]
    fn top_face_keeps_its_last_orientation() {
        let mut t = TextOrientation::new();
        t.update([1.0, 0.0, 0.0]); // +Z is a side face here
        let before = t.quarter(Face::PosZ);
        t.update([0.0, 0.0, 1.0]); // now +Z is on top
        assert_eq!(t.quarter(Face::PosZ), before);
    }

    #[test]
    fn quarter_map_rotates_clockwise() {
        // The top-left corner goes to the top-right after a clockwise turn.
        assert_eq!(Quarter::R90.map(0, 0), (95, 0));
        assert_eq!(Quarter::R180.map(0, 0), (95, 95));
        assert_eq!(Quarter::R270.map(0, 0), (0, 95));
    }
}
