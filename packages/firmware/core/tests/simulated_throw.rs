//! End-to-end: firmware core on the simulator HAL, driven by a scripted throw.

use smokebomb_core::state::Mode;
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::{imu_script, SimHandle};

#[test]
fn scripted_throw_rolls_and_shows_result() {
    let sim = SimHandle::new();
    sim.lock().manual_time_ms = Some(0);
    let mut fw = Firmware::new(sim.peripherals()).unwrap();

    {
        let mut s = sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosY);
    }

    // Run ~3 s of simulated time at 30 Hz.
    for t in 0..90u64 {
        sim.lock().manual_time_ms = Some(t * 33);
        fw.tick().unwrap();
    }

    assert!(
        matches!(fw.mode(), Mode::Reveal { .. }),
        "mode was {:?}",
        fw.mode()
    );
    let roll = fw.last_roll().expect("a roll was made");
    assert_eq!(roll.record.values.len(), 1);
    assert!((1..=20).contains(&roll.record.values[0]));

    let s = sim.lock();
    assert!(s.haptics.contains(&smokebomb_hal::HapticEffect::LandingThud));
    // The up face (PosY) shows the result; the bottom face is dark.
    assert!(s.faces[Face::PosY.index()].iter().any(|&b| b != 0));
    assert!(s.faces[Face::NegY.index()].iter().all(|&b| b == 0));
}
