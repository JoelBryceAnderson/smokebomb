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

/// Pig Toss: a throw draws two poses (always two), a tap banks the
/// turn and passes the die (on the top screen only), and the signed roll chain isn't touched.
#[test]
fn pig_toss_throws_two_pigs_and_a_tap_banks() {
    use smokebomb_core::menu::{PlayMode, Settings};
    use smokebomb_core::pigs::{Outcome, Pose};

    let sim = SimHandle::new();
    sim.lock().manual_time_ms = Some(0);
    let mut fw = Firmware::new(sim.peripherals()).unwrap();
    fw.set_settings(Settings {
        play: PlayMode::PigToss,
        players: 3,
        ..Settings::default()
    });
    assert_eq!(fw.pigs().players(), 3);

    // A word for the middle of a pose's odds.
    let word = |pose: Pose| {
        let start: u32 = Pose::ALL[..pose.index()].iter().map(|p| p.weight() as u32).sum();
        (((start as u64 * 2 + pose.weight() as u64) << 32) / 20_000) as u32
    };
    {
        let mut s = sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosY);
        s.rng_script.push_back(word(Pose::Nose));
        s.rng_script.push_back(word(Pose::Nose));
        // The pigs land apart.
        s.rng_script.push_back(u32::MAX);
    }
    let mut t = 0u64;
    let mut run = |fw: &mut Firmware<_>, ticks: u64| {
        for _ in 0..ticks {
            sim.lock().manual_time_ms = Some(t * 33);
            fw.tick().unwrap();
            t += 1;
        }
    };
    run(&mut fw, 90);

    let throw = *fw.pigs().last().expect("the throw was made");
    assert_eq!(throw.poses, [Pose::Nose, Pose::Nose]);
    assert_eq!(throw.outcome, Outcome::Score(40));
    assert_eq!(fw.pigs().turn(), 40);
    assert!(fw.last_roll().is_none(), "a pig throw isn't a signed roll");

    // A tap on a side screen does nothing: only the top one banks.
    sim.lock().touch_mask = 1 << Face::PosX.index();
    run(&mut fw, 2);
    sim.lock().touch_mask = 0;
    run(&mut fw, 2);
    assert_eq!(fw.pigs().turn(), 40, "not banked by a tap on the side");
    assert_eq!(fw.pigs().current(), 0);

    // A tap on the top screen banks it and passes to player 2.
    sim.lock().touch_mask = 1 << Face::PosY.index();
    run(&mut fw, 2);
    sim.lock().touch_mask = 0;
    run(&mut fw, 2);
    assert_eq!(fw.pigs().scores(), &[40, 0, 0]);
    assert_eq!(fw.pigs().current(), 1);
    assert_eq!(fw.pigs().turn(), 0);
}

/// Pig Toss, with the die resting after a scored throw and its top screen
/// facing +Y.
fn pig_toss_after_a_throw() -> (
    smokebomb_hal_simulator::SimHandle,
    Firmware<smokebomb_hal_simulator::SimPlatform>,
) {
    use smokebomb_core::menu::{PlayMode, Settings};
    use smokebomb_core::pigs::Pose;

    let sim = SimHandle::new();
    sim.lock().manual_time_ms = Some(0);
    let mut fw = Firmware::new(sim.peripherals()).unwrap();
    fw.set_settings(Settings {
        play: PlayMode::PigToss,
        ..Settings::default()
    });
    let word = |pose: Pose| {
        let start: u32 = Pose::ALL[..pose.index()].iter().map(|p| p.weight() as u32).sum();
        (((start as u64 * 2 + pose.weight() as u64) << 32) / 20_000) as u32
    };
    {
        let mut s = sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosY);
        s.rng_script.push_back(word(Pose::Back));
        s.rng_script.push_back(word(Pose::Back));
        // The pigs land apart.
        s.rng_script.push_back(u32::MAX);
    }
    for t in 0..150u64 {
        sim.lock().manual_time_ms = Some(t * 33);
        fw.tick().unwrap();
    }
    assert_eq!(fw.pigs().turn(), 20, "a twin belly up");
    (sim, fw)
}

/// Run `ticks` ticks from `*t`.
fn run_ticks(
    sim: &smokebomb_hal_simulator::SimHandle,
    fw: &mut Firmware<smokebomb_hal_simulator::SimPlatform>,
    t: &mut u64,
    ticks: u64,
) {
    for _ in 0..ticks {
        sim.lock().manual_time_ms = Some(*t * 33);
        fw.tick().unwrap();
        *t += 1;
    }
}

#[test]
fn picking_the_die_up_does_not_bank() {
    let (sim, mut fw) = pig_toss_after_a_throw();
    let mut t = 150;
    // A finger lands on the top screen, then the die is lifted with it.
    sim.lock().touch_mask = 1 << Face::PosY.index();
    run_ticks(&sim, &mut fw, &mut t, 3);
    sim.lock().imu_script.extend(imu_script::pick_up());
    run_ticks(&sim, &mut fw, &mut t, 40);
    sim.lock().touch_mask = 0;
    run_ticks(&sim, &mut fw, &mut t, 3);
    assert_eq!(fw.pigs().turn(), 20, "the turn wasn't banked");
    assert_eq!(fw.pigs().scores()[0], 0);
    assert_eq!(fw.pigs().current(), 0);
}

#[test]
fn a_long_press_does_not_bank_either() {
    let (sim, mut fw) = pig_toss_after_a_throw();
    let mut t = 150;
    // 0.7 s on the screen: more than a tap, less than the menu's hold.
    sim.lock().touch_mask = 1 << Face::PosY.index();
    run_ticks(&sim, &mut fw, &mut t, 21);
    sim.lock().touch_mask = 0;
    run_ticks(&sim, &mut fw, &mut t, 3);
    assert_eq!(fw.pigs().turn(), 20);
    assert_eq!(fw.pigs().current(), 0);
}

#[test]
fn a_quick_tap_on_a_resting_die_banks_and_locks_it_in() {
    let (sim, mut fw) = pig_toss_after_a_throw();
    let mut t = 150;
    sim.lock().touch_mask = 1 << Face::PosY.index();
    run_ticks(&sim, &mut fw, &mut t, 3);
    sim.lock().touch_mask = 0;
    run_ticks(&sim, &mut fw, &mut t, 3);
    assert_eq!(fw.pigs().scores()[0], 20);
    assert_eq!(fw.pigs().current(), 1);
    assert!(sim
        .lock()
        .haptics
        .contains(&smokebomb_hal::HapticEffect::LandingThud));
}
