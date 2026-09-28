//! End-to-end: firmware core on the simulator HAL, driven by a scripted throw.

use smokebomb_core::state::Mode;
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::{imu_script, SimHandle};
use smokebomb_shared::assets::SpriteKind;

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

    assert!(sim
        .lock()
        .haptics
        .contains(&smokebomb_hal::HapticEffect::LandingThud));

    // Once the landing smoke has drained (smoke may drift onto the bottom
    // face, SIM_SPEC B3), the up face (PosY) shows the result and the bottom
    // face is dark.
    for t in 90..200u64 {
        sim.lock().manual_time_ms = Some(t * 33);
        fw.tick().unwrap();
    }
    let s = sim.lock();
    assert!(s.faces[Face::PosY.index()].iter().any(|&b| b != 0));
    assert!(s.faces[Face::NegY.index()].iter().all(|&b| b == 0));
}

/// How long a tick takes with a full cloud (run with --nocapture).
#[test]
#[ignore]
fn tick_time_with_a_full_cloud() {
    let sim = SimHandle::new();
    sim.lock().manual_time_ms = Some(0);
    let mut fw = Firmware::new(sim.peripherals()).unwrap();
    {
        let mut s = sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosY);
    }
    let mut worst = std::time::Duration::ZERO;
    let start = std::time::Instant::now();
    for t in 0..240u64 {
        sim.lock().manual_time_ms = Some(t * 16);
        let t0 = std::time::Instant::now();
        fw.tick().unwrap();
        let e = t0.elapsed();
        if e.as_millis() > 16 {
            println!("tick {t}: {e:?}");
        }
        worst = worst.max(e);
    }
    println!("240 ticks in {:?}, worst {worst:?}", start.elapsed());
}

/// Throw a d20 scripted to roll `value`, and watch the smoke for 12 s:
/// returns the most gold and fizzle particles seen at once, and whether any
/// of them showed while landing smoke was still around.
fn throw_and_watch(value: u8) -> (usize, usize, bool) {
    let sim = SimHandle::new();
    sim.lock().manual_time_ms = Some(0);
    let mut fw = Firmware::new(sim.peripherals()).unwrap();
    {
        let mut s = sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosY);
        // uniform(20) maps x to x % 20 + 1.
        s.rng_script.push_back(value as u32 - 1);
    }
    let (mut gold, mut fizzle, mut early) = (0, 0, false);
    for t in 0..(12 * 60u64) {
        sim.lock().manual_time_ms = Some(t * 1000 / 60);
        fw.tick().unwrap();
        let smoke = fw.smoke_mut();
        let (g, f) = (smoke.count(SpriteKind::Gold), smoke.count(SpriteKind::Fizzle));
        if g + f > 0 && smoke.has_smoke() {
            early = true;
        }
        gold = gold.max(g);
        fizzle = fizzle.max(f);
    }
    assert_eq!(fw.last_roll().unwrap().record.values.as_slice(), &[value]);
    (gold, fizzle, early)
}

#[test]
fn a_max_roll_rings_the_die_with_gold_once_the_smoke_clears() {
    let (gold, fizzle, early) = throw_and_watch(20);
    assert_eq!((gold, fizzle), (120, 0));
    assert!(!early, "the gold waits for the smoke (SIM_SPEC C6)");
}

#[test]
fn a_dud_fizzles() {
    let (gold, fizzle, early) = throw_and_watch(1);
    assert_eq!((gold, fizzle), (0, 36));
    assert!(!early);
}

#[test]
fn an_ordinary_roll_has_no_effect() {
    assert_eq!(throw_and_watch(12), (0, 0, false));
}
