//! The firmware on the 30 mm die's 64×64 RGB panels, end to end on the
//! simulator: frames reach the panels as RGB565, only what changed is sent,
//! and the bus budget holds.

use smokebomb_core::target::{Delivery, DisplayTarget};
use smokebomb_core::Firmware;
use smokebomb_hal::{Face, Rgb64, TargetId};
use smokebomb_hal_simulator::{imu_script, SimHandle, SimPlatform};

fn boot() -> (SimHandle, Firmware<SimPlatform<Rgb64>>) {
    let sim = SimHandle::new();
    {
        let mut s = sim.lock();
        s.manual_time_ms = Some(0);
        s.imu_resting = imu_script::resting(Face::PosY);
    }
    let fw = Firmware::new(sim.peripherals_for::<Rgb64>()).unwrap();
    (sim, fw)
}

fn run(fw: &mut Firmware<SimPlatform<Rgb64>>, sim: &SimHandle, t: &mut u64, seconds: f64) {
    for _ in 0..(seconds * 60.0) as u64 {
        *t += 17;
        sim.lock().manual_time_ms = Some(*t);
        fw.tick().unwrap();
    }
}

#[test]
fn frames_arrive_as_rgb565_and_only_changes_are_sent() {
    let (sim, mut fw) = boot();
    let mut t = 0;
    run(&mut fw, &sim, &mut t, 1.0);
    {
        let s = sim.lock();
        assert_eq!(s.target, Some(TargetId::Rgb64));
        assert!(s.faces.iter().all(|f| f.len() == 64 * 64 * 2));
        assert!(s.faces.iter().any(|f| f.iter().any(|&b| b != 0)), "boot shows");
    }
    // Let the boot finish and the wake label go: the faces settle.
    run(&mut fw, &sim, &mut t, 9.0);
    let before = sim.lock().panel_bytes;
    run(&mut fw, &sim, &mut t, 2.0);
    let after = sim.lock().panel_bytes;
    let sent: u64 = (0..6).map(|f| after[f] - before[f]).sum();
    assert_eq!(sent, 0, "a still die sends nothing");
    assert_eq!(fw.panel_bytes(), after, "the firmware and the bus agree");
}

#[test]
fn a_throw_stays_within_the_bus_budget() {
    let Delivery::Dirty { bytes_per_tick, .. } = <Rgb64 as DisplayTarget>::DELIVERY else {
        unreachable!()
    };
    let (sim, mut fw) = boot();
    let mut t = 0;
    run(&mut fw, &sim, &mut t, 7.0);
    sim.lock().imu_script.extend(imu_script::throw());
    let mut worst = 0;
    let mut total = 0;
    for _ in 0..(4 * 60) {
        let before: u64 = sim.lock().panel_bytes.iter().sum();
        run(&mut fw, &sim, &mut t, 1.0 / 60.0);
        let tick = sim.lock().panel_bytes.iter().sum::<u64>() - before;
        worst = worst.max(tick);
        total += tick;
    }
    // A single face may exceed the budget only when it is all that goes.
    assert!(worst as usize <= bytes_per_tick.max(8192), "worst tick {worst} B");
    assert!(total > 0, "the throw drew");
    println!("throw: {total} B over 4 s, worst tick {worst} B, budget {bytes_per_tick} B");
}
