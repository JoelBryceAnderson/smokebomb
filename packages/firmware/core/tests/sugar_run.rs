//! Sugar Run end to end: the firmware on the simulator HAL, with the die
//! moved by the simulator's world model.

use smokebomb_core::maze::{self, Phase};
use smokebomb_core::menu::{PlayMode, Settings};
use smokebomb_core::Firmware;
use smokebomb_hal::Face;
use smokebomb_hal_simulator::world::{face_normal, World, DEFAULT_VIEWER_RIGHT};
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
        Self::with_maze(1)
    }

    /// The maze is drawn from the TRNG's next word: `seed`.
    fn with_maze(seed: u32) -> Self {
        let sim = SimHandle::new();
        sim.lock().manual_time_ms = Some(0);
        let mut fw = Firmware::new(sim.peripherals()).unwrap();
        fw.set_settings(Settings {
            play: PlayMode::SugarRun,
            ..Settings::default()
        });
        sim.lock().rng_script.push_back(seed);
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

    /// Lean the die so the side of `way` goes down, as the viewer sees it.
    fn lean_toward(&mut self, way: Face) {
        let v = self.world.pose().rotation * face_normal(way);
        let right = DEFAULT_VIEWER_RIGHT;
        let front = right.cross(glam::Vec3::Y);
        let (r, a) = (v.dot(right), -v.dot(front));
        let k = LEAN / r.abs().max(a.abs()).max(1e-3);
        self.world.tilt(r * k, a * k, right);
    }

    /// An open way out of the runner's cell, other than `not`.
    fn open_way(&self, not: Option<Face>) -> Face {
        let r = self.fw.run().runner;
        maze::ways(r.cell.face())
            .into_iter()
            .find(|d| self.fw.run().maze().open(r.cell, *d) && Some(*d) != not)
            .expect("no dead ends")
    }

    /// Wait for the runner to stop at a wall, lean toward an open way, and
    /// say whether it went that way.
    fn steers(&mut self) -> bool {
        while self.fw.run().runner.moving && self.phase() == Phase::Playing {
            self.step();
        }
        let from = self.fw.run().runner.cell;
        let way = self.open_way(None);
        self.lean_toward(way);
        for _ in 0..60 {
            self.step();
            if self.fw.run().runner.cell != from {
                break;
            }
        }
        self.fw.run().runner.cell == from.step(way).0
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
        rig.fw.run().runner.cell.face(),
        Face::PosY,
        "on the screen that's up"
    );
    while rig.fw.run().runner.moving {
        rig.step();
    }
    assert_eq!(
        rig.fw.run().runner.face(),
        Face::PosY,
        "stopped on the top screen"
    );
    assert!(rig.steers(), "it goes the way the die leans");
}

#[test]
fn off_the_top_you_tip_the_die_to_follow() {
    let mut rig = Rig::new();
    rig.start();
    // Steer it wherever it can go until it leaves the top screen.
    let mut t = 0.0;
    while rig.fw.run().runner.face() == Face::PosY && t < 20.0 {
        if !rig.fw.run().runner.moving {
            let back = rig.fw.run().runner.dir.opposite();
            let way = rig.open_way(Some(back));
            rig.lean_toward(way);
        }
        rig.run_for(0.1);
        t += 0.1;
    }
    let side = rig.fw.run().runner.face();
    assert_ne!(side, Face::PosY, "it ran off the top");
    // Its screen faces sideways: leaning doesn't steer it there.
    assert!(!rig.steers(), "no steering on a screen facing sideways");
    // Tip its screen up, and leaning steers it again.
    rig.world.place_face_up(rig.fw.run().runner.face());
    rig.run_for(0.8);
    assert_eq!(rig.phase(), Phase::Playing);
    assert!(rig.steers(), "steered on its new screen");
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
