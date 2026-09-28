//! The firmware on a clock that moves on while a tick runs, as a real one
//! does: everything in one tick must use that tick's time.

use smokebomb_core::state::Mode;
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::{imu_script, SimHandle};

#[test]
fn a_clock_that_moves_during_a_tick_is_harmless() {
    let sim = SimHandle::new();
    {
        let mut s = sim.lock();
        s.manual_time_ms = Some(0);
        s.imu_resting = imu_script::resting(Face::PosY);
        s.clock_drift_per_read_ms = 1;
    }
    let mut fw = Firmware::new(sim.peripherals()).unwrap();
    let mut t = 0u64;
    let mut run = |fw: &mut Firmware<_>, sim: &SimHandle, seconds: f64| {
        for _ in 0..(seconds * 60.0) as u64 {
            t += 17;
            sim.lock().manual_time_ms = Some(t);
            fw.tick().unwrap();
        }
    };
    run(&mut fw, &sim, 7.0); // boot
                             // Hold to open the menu, let go, hold again to save: closing the menu
                             // starts the setup label mid-tick.
    for expect in [Mode::Menu, Mode::Idle] {
        sim.lock().touch_mask = 1 << Face::PosZ.index();
        run(&mut fw, &sim, 1.2);
        sim.lock().touch_mask = 0;
        run(&mut fw, &sim, 0.5);
        assert_eq!(*fw.mode(), expect);
    }
    run(&mut fw, &sim, 3.0);
}
