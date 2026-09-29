//! IMU gesture detection: turns raw samples into coarse motion events.
//!
//! Thresholds are first guesses for an LSM6DSx at +/-16 g and need tuning on
//! real hardware with recorded throws.

use smokebomb_hal::{Face, ImuSample};

/// Coarse motion classification fed to the state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    /// Picked up or moved gently.
    Handled,
    /// Vigorous shaking in the hand.
    Shaking,
    /// Near-zero g: the die has left the hand.
    FreeFall,
    /// Hit the table.
    Impact,
    /// Motionless for [`REST_MS`].
    Rest,
}

const ONE_G_MG: i32 = 1000;
const FREE_FALL_MG: i32 = 350;
const IMPACT_MG: i32 = 2_500;
const SHAKE_DEVIATION_MG: i32 = 700;
const STILL_DEVIATION_MG: i32 = 80;
const STILL_GYRO_MDPS: i32 = 15_000;
/// Stopped outright: what a thrown die reads once it has landed. A settled
/// die reads well under this once the gyro's bias is calibrated out; if it
/// never does, the landing falls back to the moment the die is at rest.
const STOPPED_GYRO_MDPS: i32 = 1_000;
/// How long the die must be motionless before it counts as at rest.
pub const REST_MS: u64 = 300;

pub struct MotionDetector {
    last: Option<Motion>,
    still_since: Option<u64>,
    stopped: bool,
}

impl Default for MotionDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionDetector {
    pub const fn new() -> Self {
        Self {
            last: None,
            still_since: None,
            stopped: false,
        }
    }

    /// Stopped outright on the last sample: the moment a thrown die has
    /// landed, before it counts as at rest.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    pub fn is_still(&self) -> bool {
        matches!(self.last, None | Some(Motion::Rest))
    }

    /// Feed a sample; returns a motion event when the classification changes.
    pub fn update(&mut self, s: &ImuSample, now_ms: u64) -> Option<Motion> {
        let mag = magnitude_mg(s);
        let gyro = s.gyro_mdps.iter().map(|g| g.abs()).max().unwrap_or(0);
        self.stopped = (mag - ONE_G_MG).abs() <= STILL_DEVIATION_MG && gyro <= STOPPED_GYRO_MDPS;

        let instant = if mag < FREE_FALL_MG {
            Some(Motion::FreeFall)
        } else if mag > IMPACT_MG {
            Some(Motion::Impact)
        } else if (mag - ONE_G_MG).abs() > SHAKE_DEVIATION_MG {
            Some(Motion::Shaking)
        } else if (mag - ONE_G_MG).abs() <= STILL_DEVIATION_MG && gyro <= STILL_GYRO_MDPS {
            None // candidate for rest, decided below
        } else {
            Some(Motion::Handled)
        };

        let next = match instant {
            Some(m) => {
                self.still_since = None;
                // Shaking in the hand produces moments of ~1 g; don't flap
                // back to Handled mid-shake.
                if m == Motion::Handled && self.last == Some(Motion::Shaking) {
                    return None;
                }
                m
            }
            None => {
                let since = *self.still_since.get_or_insert(now_ms);
                if now_ms - since < REST_MS {
                    return None;
                }
                Motion::Rest
            }
        };

        if self.last == Some(next) {
            None
        } else {
            self.last = Some(next);
            Some(next)
        }
    }
}

/// Integer |a| in milli-g.
pub fn magnitude_mg(s: &ImuSample) -> i32 {
    let sq: i64 = s.accel_mg.iter().map(|&a| (a as i64) * (a as i64)).sum();
    isqrt(sq as u64) as i32
}

/// The face pointing up, from gravity. `None` while the reading is ambiguous
/// (die on an edge or in motion).
pub fn up_face(s: &ImuSample) -> Option<Face> {
    let [x, y, z] = s.accel_mg.map(|a| a as i32);
    let (axis, value) = [(0, x), (1, y), (2, z)]
        .into_iter()
        .max_by_key(|(_, v)| v.abs())?;
    if value.abs() < 700 {
        return None;
    }
    Some(match (axis, value > 0) {
        (0, true) => Face::PosX,
        (0, false) => Face::NegX,
        (1, true) => Face::PosY,
        (1, false) => Face::NegY,
        (_, true) => Face::PosZ,
        (_, false) => Face::NegZ,
    })
}

fn isqrt(n: u64) -> u64 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accel(x: i16, y: i16, z: i16) -> ImuSample {
        ImuSample {
            accel_mg: [x, y, z],
            gyro_mdps: [0; 3],
        }
    }

    #[test]
    fn detects_rest_after_delay() {
        let mut d = MotionDetector::new();
        assert_eq!(d.update(&accel(0, 0, 1000), 0), None);
        assert_eq!(d.update(&accel(0, 0, 1000), REST_MS), Some(Motion::Rest));
    }

    #[test]
    fn throw_sequence() {
        let mut d = MotionDetector::new();
        assert_eq!(d.update(&accel(1800, 900, 200), 0), Some(Motion::Shaking));
        assert_eq!(d.update(&accel(0, 0, 100), 10), Some(Motion::FreeFall));
        assert_eq!(d.update(&accel(3000, 0, 0), 20), Some(Motion::Impact));
    }

    #[test]
    fn up_face_from_gravity() {
        assert_eq!(up_face(&accel(0, 0, 1000)), Some(Face::PosZ));
        assert_eq!(up_face(&accel(-990, 30, 0)), Some(Face::NegX));
        assert_eq!(up_face(&accel(600, 600, 0)), None);
    }
}
