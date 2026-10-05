//! The phone decides which licensed modes the die offers.

use smokebomb_core::menu::{PlayMode, Settings};
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::{imu_script, SimHandle};
use smokebomb_shared::{ModeId, ModeSet};

fn boot() -> (SimHandle, Firmware<smokebomb_hal_simulator::SimPlatform>) {
    let sim = SimHandle::new();
    {
        let mut s = sim.lock();
        s.manual_time_ms = Some(0);
        s.imu_resting = imu_script::resting(Face::PosY);
    }
    let fw = Firmware::new(sim.peripherals()).unwrap();
    (sim, fw)
}

#[test]
fn turning_off_the_current_mode_goes_back_to_dice() {
    let (_sim, mut fw) = boot();
    fw.set_settings(Settings {
        play: PlayMode::HotPotato,
        ..Settings::default()
    });
    assert_eq!(fw.inventory().active, ModeId::HotPotato);
    assert_eq!(fw.inventory().enabled, ModeSet::ALL);

    fw.set_enabled_modes(ModeSet::EMPTY.with(ModeId::PigToss));
    let inv = fw.inventory();
    assert_eq!(inv.enabled, ModeSet::DICE.with(ModeId::PigToss), "Dice stays on");
    assert_eq!(inv.active, ModeId::Dice);
    assert_eq!(fw.settings().play(), PlayMode::Dice);
}

#[test]
fn licenses_survive_replaced_settings() {
    let (_sim, mut fw) = boot();
    fw.set_settings(Settings {
        licensed: ModeSet::DICE,
        ..Settings::default()
    });
    assert_eq!(
        fw.inventory().licensed,
        ModeSet::ALL,
        "settings can't grant or drop licenses"
    );
    fw.set_enabled_modes(ModeSet::DICE);
    assert_eq!(fw.settings().modes(), ModeSet::DICE);
    fw.unlock_mode(ModeId::PigToss);
    assert!(
        fw.inventory().enabled.contains(ModeId::PigToss),
        "a new mode starts on"
    );
    assert!(fw.settings().modes().contains(ModeId::PigToss));
}
