//! Sugar Run end to end: the firmware on the simulator HAL, with the die
//! moved by the simulator's world model.

use smokebomb_core::maze::Phase;
use smokebomb_core::menu::{PlayMode, Settings};
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::world::{World, DEFAULT_VIEWER_RIGHT};
use smokebomb_hal_simulator::{SimHandle, SimPlatform};

const FPS: f64 = 60.0;
const LEAN: f32 = 0.35;

struct Rig {
    sim: SimHandle,
    fw: Firmware<SimPlatform>,
    world: World,
    frame: u64,
}

impl Rig {
    fn new() -> Self {
        let sim = SimHandle::new();
        sim.lock().manual_time_ms = Some(0);
        let mut fw = Firmware::new(sim.peripherals()).unwrap();
        fw.set_settings(Settings {
            play: PlayMode::SugarRun,
            ..Settings::default()
        });
        let mut world = World::with_seed(3);
        world.set_next_landing(Face::PosY);
        Self {
            sim,
            fw,
            world,
            frame: 0,
        }
    }

    fn step(&mut self) {
        {
            let mut s = self.sim.lock();
            s.manual_time_ms = Some((self.frame as f64 * 1000.0 / FPS).round() as u64);
            let dt = if self.frame == 0 { 0.0 } else { 1.0 / FPS };
            s.imu_resting = self.world.step(dt);
            s.sync_world(&self.world, dt);
        }
        self.fw.tick().unwrap();
        self.frame += 1;
    }

    fn run_for(&mut self, secs: f64) {
        for _ in 0..(secs * FPS) as u64 {
            self.step();
        }
    }

    fn touch(&mut self, face: Face, on: bool) {
        self.sim.lock().touch_mask = if on { 1 << face.index() } else { 0 };
    }

    fn phase(&self) -> Phase {
        self.fw.run().phase()
    }

    /// Past the boot, shake and throw: the run starts once the die settles.
    fn start(&mut self) {
        self.run_for(8.0);
        assert_eq!(self.phase(), Phase::Ready, "a shake starts it");
        self.world.start_shake();
        self.run_for(0.4);
        self.world.end_shake(true);
        self.run_for(0.5);
        assert_eq!(self.phase(), Phase::Ready, "not while it tumbles");
        self.run_for(1.5);
        assert_eq!(self.phase(), Phase::Playing);
    }
}

#[test]
fn a_run_starts_when_the_shake_settles_and_steers_by_the_lean() {
    let mut rig = Rig::new();
    rig.start();
    assert_eq!(
        rig.fw.run_view().turn().maze_face(Face::PosY),
        Face::PosY,
        "the runner's face on top"
    );
    // Held leaning, the run goes on and the runner goes somewhere.
    rig.world.tilt(LEAN, 0.0, DEFAULT_VIEWER_RIGHT);
    let score = rig.fw.run().score();
    rig.run_for(3.0);
    assert!(matches!(rig.phase(), Phase::Playing | Phase::Caught { .. }));
    assert!(rig.fw.run().score() > score, "it ate");
    // Whatever face the runner is on is on top.
    let runner = rig.fw.run().runner.face();
    if !rig.fw.run_view().rolling() {
        assert_eq!(rig.fw.run_view().turn().maze_face(Face::PosY), runner);
    }
}

#[test]
fn a_hold_mid_run_is_the_grip_and_setting_it_down_pauses() {
    let mut rig = Rig::new();
    rig.start();
    rig.world.tilt(LEAN, 0.0, DEFAULT_VIEWER_RIGHT);
    rig.run_for(0.5);
    // Fingers on the screens while it's leaning: no menu.
    rig.touch(Face::PosZ, true);
    rig.run_for(1.5);
    rig.touch(Face::PosZ, false);
    rig.run_for(0.1);
    assert!(
        rig.fw.menu_draft().is_none(),
        "a hold mid-run doesn't open the menu"
    );
    assert!(rig.fw.run().in_play());
    // Set down level: it pauses after a while, not at once.
    rig.world.tilt(0.0, 0.0, DEFAULT_VIEWER_RIGHT);
    rig.run_for(2.0);
    assert!(!matches!(rig.phase(), Phase::Paused), "not straight away");
    rig.run_for(4.0);
    assert_eq!(rig.phase(), Phase::Paused);
    let runner = rig.fw.run().runner;
    rig.run_for(1.0);
    assert_eq!(rig.fw.run().runner, runner, "paused, nothing moves");
    // Tilted again, it carries on.
    rig.world.tilt(0.0, LEAN, DEFAULT_VIEWER_RIGHT);
    rig.run_for(0.6);
    assert!(matches!(rig.phase(), Phase::Playing | Phase::Caught { .. }));
}

#[test]
fn paused_the_menu_opens_and_saving_it_keeps_the_run() {
    let mut rig = Rig::new();
    rig.start();
    // Lying level on the table from the start: paused within a few seconds.
    rig.run_for(4.5);
    assert_eq!(rig.phase(), Phase::Paused);
    let score = rig.fw.run().score();
    rig.touch(Face::PosZ, true);
    rig.run_for(1.0);
    rig.touch(Face::PosZ, false);
    rig.run_for(0.5);
    assert!(
        rig.fw.menu_draft().is_some(),
        "a hold opens the menu while paused"
    );
    // Hold again to save without changing anything: the run is kept.
    rig.touch(Face::PosZ, true);
    rig.run_for(1.0);
    rig.touch(Face::PosZ, false);
    rig.run_for(0.5);
    assert!(rig.fw.menu_draft().is_none());
    assert_eq!(rig.phase(), Phase::Paused);
    assert_eq!(rig.fw.run().score(), score);
    // A different number of ants is a new game.
    let settings = Settings {
        ants: 1,
        ..*rig.fw.settings()
    };
    rig.fw.set_settings(settings);
    assert_eq!(rig.phase(), Phase::Ready);
}
