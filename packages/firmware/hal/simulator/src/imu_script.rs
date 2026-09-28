//! Canned IMU sequences so the simulator can "throw" the die with one click.
//!
//! Sequences are sampled at the firmware tick rate (one sample per tick).

use smokebomb_hal::{Face, ImuSample};

/// Gravity reading with `up` facing the ceiling.
pub fn resting(up: Face) -> ImuSample {
    let g = 1000i16;
    let accel_mg = match up {
        Face::PosX => [g, 0, 0],
        Face::NegX => [-g, 0, 0],
        Face::PosY => [0, g, 0],
        Face::NegY => [0, -g, 0],
        Face::PosZ => [0, 0, g],
        Face::NegZ => [0, 0, -g],
    };
    ImuSample {
        accel_mg,
        gyro_mdps: [0; 3],
    }
}

fn sample(x: i16, y: i16, z: i16, gyro: i32) -> ImuSample {
    ImuSample {
        accel_mg: [x, y, z],
        gyro_mdps: [gyro, -gyro / 2, gyro / 3],
    }
}

/// Pick up and wobble in the hand.
pub fn pick_up() -> Vec<ImuSample> {
    (0..6).map(|i| sample(200 + i * 20, 150, 1150, 60_000)).collect()
}

/// Roughly one second of vigorous shaking.
pub fn shake() -> Vec<ImuSample> {
    (0..30)
        .map(|i| {
            let s = if i % 2 == 0 { 1 } else { -1 };
            sample(s * 1_900, -s * 600, 900, 400_000)
        })
        .collect()
}

/// Full throw: shake, release, flight, impact, tumble. The caller should set
/// the resting orientation to the landing face afterwards.
pub fn throw() -> Vec<ImuSample> {
    let mut v = pick_up();
    v.extend(shake());
    v.extend((0..9).map(|_| sample(30, -20, 60, 900_000))); // ~300 ms airborne
    v.push(sample(4_200, -1_500, 2_800, 200_000)); // impact
    v.extend((0..8).map(|i| sample(400 - i * 40, 300, 1_300 - i * 30, 120_000))); // tumble
    v
}
