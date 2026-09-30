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

    /// The die direction a way through the maze stands for, as the map sits.
    fn die_way(&self, way: Face) -> Face {
        let v = self.fw.run_view().turn().unapply(maze::axis(way));
        Face::ALL.into_iter().find(|f| maze::axis(*f) == v).unwrap()
    }

    /// Lean the die so the side of the maze way `way` goes down, as the
    /// viewer sees it, from however it's held.
    fn lean_toward(&mut self, way: Face) {
        let v = self.world.pose().rotation * face_normal(self.die_way(way));
        let right = DEFAULT_VIEWER_RIGHT;
        let front = right.cross(glam::Vec3::Y);
        let (r, a) = (v.dot(right), -v.dot(front));
        let k = LEAN / r.abs().max(a.abs()).max(1e-3);
        self.world.tilt(r * k, a * k, right);
    }

    /// Wait for the runner to stop at a wall, lean toward an open way, and
    /// say whether it went that way.
    fn steers(&mut self) -> bool {
        while self.fw.run().runner.moving && self.phase() == Phase::Playing {
            self.step();
        }
        let r = self.fw.run().runner;
        let way = maze::ways(r.cell.face())
            .into_iter()
            .find(|d| self.fw.run().maze().open(r.cell, *d))
            .expect("no dead ends");
        self.lean_toward(way);
        for _ in 0..60 {
            self.step();
            if self.fw.run().runner.cell != r.cell {
                break;
            }
        }
        self.world.tilt(0.0, 0.0, DEFAULT_VIEWER_RIGHT);
        self.fw.run().runner.cell == r.cell.step(way).0
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
    rig.run_for(0.5);
    assert!(rig.steers(), "it goes the way the die leans");
}

#[test]
fn held_up_toward_you_it_steers_from_there() {
    // Held with the top screen tipped about 40° toward you, as to watch it:
    // that's neutral, not a lean toward you.
    let mut rig = Rig::new();
    rig.start();
    rig.world.set_held(true);
    rig.run_for(1.5);
    assert_eq!(rig.fw.run_want(), None, "being held up isn't steering");
    while rig.fw.run_view().rolling() {
        rig.step();
    }
    assert_eq!(
        rig.fw.run_view().turn().maze_face(Face::PosY),
        rig.fw.run().runner.face(),
        "held up, the runner stays on the screen it started on"
    );
    assert!(rig.steers(), "a lean from the held pose steers");
    rig.run_for(0.5);
    assert!(rig.steers(), "and again");
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
