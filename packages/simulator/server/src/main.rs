//! Smokebomb simulator.
//!
//! Boots the firmware core on the simulator HAL, ticks it at the firmware
//! rate, and bridges it to the browser over a WebSocket:
//!
//! * server -> browser: binary frame packets (all six faces) and JSON events
//! * browser -> server: JSON input (touch, orientation, gestures, docking)
//!
//! `cargo run --features=simulator` (or `cargo sim`) then open
//! <http://localhost:3000>.

mod protocol;
mod ws;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::routing::get;
use axum::{Json, Router};
use smokebomb_firmware::board::{self, SimHandle};
use smokebomb_hal::SecureElement;
use smokebomb_hal_simulator::SimSecureElement;
use tokio::sync::{broadcast, Mutex};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;

use protocol::{Outbound, StatusSnapshot};
use smokebomb_hal_simulator::world;

#[derive(Clone)]
pub struct AppState {
    pub sim: SimHandle,
    /// The physical die. Locked briefly by the tick loop and by input.
    pub world: Arc<std::sync::Mutex<world::World>>,
    pub out: broadcast::Sender<Outbound>,
    pub status: Arc<Mutex<StatusSnapshot>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "smokebomb_simulator=info,tower_http=info".into()),
        )
        .init();

    let sim = SimHandle::new();
    let firmware = board::boot(&sim).map_err(|e| anyhow::anyhow!("firmware boot failed: {e:?}"))?;
    let (out, _) = broadcast::channel(64);
    let state = AppState {
        sim: sim.clone(),
        world: Arc::new(std::sync::Mutex::new(world::World::new())),
        out: out.clone(),
        // Seeded from the booted firmware, so a browser that connects before
        // the first tick still gets the real mode in its hello.
        status: Arc::new(Mutex::new(StatusSnapshot {
            mode: format!("{:?}", firmware.mode()),
            die: firmware.settings().active().0.wire_name(),
            die_count: firmware.settings().active().1,
            last_roll: None,
        })),
    };

    tokio::spawn(run_firmware(firmware, state.clone()));

    let web_dir = std::env::var_os("SMOKEBOMB_WEB_UI_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../web-ui/dist")));
    if !web_dir.join("index.html").exists() {
        tracing::warn!(
            "web UI not built at {} — run `npm run build -w @smokebomb/simulator-web-ui`, \
             or use the Vite dev server on http://localhost:5173",
            web_dir.display()
        );
    } else if web_ui_is_stale(&web_dir) {
        tracing::warn!(
            "the web UI build at {} is older than its sources, so the browser may run an old UI \
             that this simulator doesn't understand. Rebuild it: \
             `npm run build -w @smokebomb/simulator-web-ui` (or use `npx nx run simulator-server:serve`)",
            web_dir.display()
        );
    }
    // Always revalidate: the UI is rebuilt often, and a browser that keeps a
    // cached index.html (Safari does, without this) runs an old bundle.
    let static_files = SetResponseHeader::overriding(
        ServeDir::new(&web_dir).fallback(ServeFile::new(web_dir.join("index.html"))),
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );

    let app = Router::new()
        .route("/ws", get(ws::handler))
        .route("/api/state", get(get_state))
        .route("/api/device", get(get_device))
        .fallback_service(static_files)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    // Loopback by default. HOST=0.0.0.0 exposes the simulator (which has no
    // auth) to the local network, e.g. to drive it from an iPad.
    let host: IpAddr = match std::env::var("HOST") {
        Ok(h) => h
            .parse()
            .map_err(|_| anyhow::anyhow!("HOST must be an IP address, got {h:?}"))?,
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    };
    let addr = SocketAddr::new(host, port);
    if host.is_loopback() {
        tracing::info!("simulator running on http://localhost:{port}");
    } else {
        tracing::warn!("simulator listening on {addr} and reachable from the network (no authentication)");
        tracing::info!("open http://<this-machine>.local:{port} from another device");
    }
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// One firmware tick, in seconds: exactly 1/TICK_HZ, so the world's IMU
/// samples cover the period the firmware integrates them over.
pub const TICK_S: f64 = 1.0 / smokebomb_firmware::smokebomb_core::TICK_HZ as f64;

/// Firmware main loop: tick, then publish anything that changed.
async fn run_firmware(mut fw: board::Firmware, state: AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs_f64(TICK_S));
    let mut last_seq = 0;
    let mut last_mode = String::new();
    let mut last_counter = None;
    let mut last_pose = None;
    let mut menu_was_open = false;
    let dt = TICK_S;

    loop {
        interval.tick().await;

        // Move the die first, so the firmware reads this tick's IMU sample.
        let (pose, imu, docked) = {
            let mut w = state.world.lock().unwrap_or_else(|e| e.into_inner());
            let imu = w.step(dt);
            (w.pose(), imu, w.docked())
        };
        {
            let mut s = state.sim.lock();
            s.imu_resting = imu;
            s.docked = docked;
        }
        if last_pose != Some(pose) {
            last_pose = Some(pose);
            let _ = state.out.send(Outbound::Binary(protocol::encode_pose(&pose)));
        }

        if let Err(e) = fw.tick() {
            tracing::error!("firmware tick failed: {e:?}");
            continue;
        }

        // The menu just opened: turn the held face toward the viewer, as the
        // person would (the mockup does the same).
        let menu_front = fw.menu_front();
        if let (false, Some(face)) = (menu_was_open, menu_front) {
            state
                .world
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .snap_to_viewer(face);
        }
        menu_was_open = menu_front.is_some();

        let (frames, haptics) = {
            let mut s = state.sim.lock();
            let frames = (s.frame_seq != last_seq).then(|| {
                last_seq = s.frame_seq;
                protocol::encode_frames(&s.faces)
            });
            (frames, s.haptics.drain(..).collect::<Vec<_>>())
        };
        // Send errors only mean no browser is connected.
        if let Some(f) = frames {
            let _ = state.out.send(Outbound::Binary(f));
        }
        for h in haptics {
            let _ = state.out.send(Outbound::event(protocol::Event::Haptic {
                effect: format!("{h:?}"),
            }));
        }

        // "Reveal { since_ms: 123 }" -> "Reveal".
        let mode = format!("{:?}", fw.mode())
            .split(" {")
            .next()
            .unwrap_or_default()
            .to_string();
        if mode != last_mode {
            last_mode.clone_from(&mode);
            let _ = state
                .out
                .send(Outbound::event(protocol::Event::Mode { mode: mode.clone() }));
        }

        let roll = fw
            .last_roll()
            .filter(|r| Some(r.record.counter) != last_counter)
            .cloned();
        if let Some(roll) = &roll {
            last_counter = Some(roll.record.counter);
            let _ = state
                .out
                .send(Outbound::event(protocol::Event::Roll(roll.into())));
        }

        let mut status = state.status.lock().await;
        status.mode = mode;
        let (die, count) = fw.settings().active();
        status.die = die.wire_name();
        status.die_count = count;
        if let Some(roll) = roll {
            status.last_roll = Some((&roll).into());
        }
    }
}

/// True when any web UI source file is newer than the built `index.html`.
/// Only checks the usual layout (`web-ui/dist` next to `web-ui/src`).
fn web_ui_is_stale(dist: &std::path::Path) -> bool {
    fn newest(path: &std::path::Path) -> Option<std::time::SystemTime> {
        let meta = std::fs::metadata(path).ok()?;
        if meta.is_dir() {
            std::fs::read_dir(path)
                .ok()?
                .filter_map(|e| newest(&e.ok()?.path()))
                .max()
        } else {
            meta.modified().ok()
        }
    }
    let Some(root) = dist.parent() else {
        return false;
    };
    let built = std::fs::metadata(dist.join("index.html")).and_then(|m| m.modified());
    let sources = ["src", "index.html", "package.json", "vite.config.ts"]
        .iter()
        .filter_map(|p| newest(&root.join(p)))
        .max();
    matches!((built, sources), (Ok(b), Some(s)) if s > b)
}

async fn get_state(State(state): State<AppState>) -> Json<StatusSnapshot> {
    Json(state.status.lock().await.clone())
}

/// Identity of the simulated die, for registering it with the backend.
async fn get_device() -> Json<serde_json::Value> {
    let mut se = SimSecureElement::new();
    let serial = se.serial().expect("sim serial");
    let key = se.public_key().expect("sim public key");
    Json(serde_json::json!({
        "serial": hex::encode(serial),
        "public_key": hex::encode(key),
        "firmware_version": env!("CARGO_PKG_VERSION"),
    }))
}

#[cfg(test)]
mod tests {
    use super::world::World;

    use super::TICK_S as DT;

    /// The real firmware, fed only by this world's IMU, rolls after a throw.
    #[test]
    fn firmware_rolls_from_a_simulated_throw() {
        use smokebomb_firmware::board;
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let sim = board::SimHandle::new();
        sim.lock().manual_time_ms = Some(0);
        let mut fw = board::boot(&sim).unwrap();
        let mut w = World::new();
        let mut t = 0.0;
        let mut tick = |w: &mut World, fw: &mut board::Firmware| {
            t += DT;
            let imu = w.step(DT);
            {
                let mut s = sim.lock();
                s.imu_resting = imu;
                s.manual_time_ms = Some((t * 1000.0) as u64);
            }
            fw.tick().unwrap();
        };
        for _ in 0..30 {
            tick(&mut w, &mut fw);
        }
        w.start_shake();
        for _ in 0..30 {
            tick(&mut w, &mut fw);
        }
        w.end_shake(true);
        for _ in 0..150 {
            tick(&mut w, &mut fw);
        }
        assert!(matches!(fw.mode(), Mode::Reveal { .. }), "{:?}", fw.mode());
        assert!(fw.last_roll().is_some());
    }

    /// The firmware on the world model, ticked like `run_firmware` does.
    struct Rig {
        sim: board::SimHandle,
        fw: board::Firmware,
        world: World,
        t: f64,
        menu_was_open: bool,
    }

    use smokebomb_firmware::board;
    use smokebomb_hal::Face;
    use smokebomb_hal_simulator::world::{Hand, SpinAxis, TipDir, DEFAULT_VIEWER_RIGHT};

    impl Rig {
        fn new() -> Self {
            Self::with_world(World::new())
        }

        fn with_world(world: World) -> Self {
            let sim = board::SimHandle::new();
            sim.lock().manual_time_ms = Some(0);
            let fw = board::boot(&sim).unwrap();
            Self {
                sim,
                fw,
                world,
                t: 0.0,
                menu_was_open: false,
            }
        }

        fn run(&mut self, seconds: f64) {
            for _ in 0..(seconds / DT).round() as usize {
                self.t += DT;
                let imu = self.world.step(DT);
                {
                    let mut s = self.sim.lock();
                    s.imu_resting = imu;
                    s.manual_time_ms = Some((self.t * 1000.0) as u64);
                }
                self.fw.tick().unwrap();
                let front = self.fw.menu_front();
                if let (false, Some(face)) = (self.menu_was_open, front) {
                    self.world.snap_to_viewer(face);
                }
                self.menu_was_open = front.is_some();
            }
        }

        fn hold(&mut self, face: Face) {
            self.sim.lock().touch_mask = 1 << face.index();
            self.run(1.0);
            self.sim.lock().touch_mask = 0;
            self.run(0.5);
        }

        fn tap(&mut self, face: Face) {
            self.sim.lock().touch_mask = 1 << face.index();
            self.run(0.15);
            self.sim.lock().touch_mask = 0;
            self.run(0.3);
        }

        fn tip(&mut self, dir: TipDir) {
            assert!(self.world.tip(dir, DEFAULT_VIEWER_RIGHT));
            self.run(0.6);
        }

        /// A multi-turn drag: spin to `quarters` faces (fractional is fine,
        /// pausing on the way), then let go.
        fn spin(&mut self, axis: SpinAxis, quarters: f32) {
            let steps = 30;
            for i in 1..=steps {
                let angle = quarters * std::f32::consts::FRAC_PI_2 * i as f32 / steps as f32;
                assert!(self.world.spin(axis, angle, DEFAULT_VIEWER_RIGHT));
                self.run(DT);
                if i == steps / 2 {
                    self.run(0.4); // a pause between faces
                }
            }
            self.run(0.2);
            self.world.end_spin();
            self.run(0.8);
        }
    }

    /// A slow drag with long stops (a busy browser sends pointer moves this
    /// sparsely): still three values, and one tick per face, not per nudge.
    #[test]
    fn a_stop_start_multi_turn_counts_faces_and_ticks_once_per_face() {
        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.sim.lock().haptics.clear();
        for i in 1..=30 {
            let angle = -3.0 * std::f32::consts::FRAC_PI_2 * i as f32 / 30.0;
            assert!(rig.world.spin(SpinAxis::Pitch, angle, DEFAULT_VIEWER_RIGHT));
            rig.run(0.45);
        }
        rig.world.end_spin();
        rig.run(1.5);
        assert_eq!(rig.fw.menu_draft().unwrap().count, 4);
        let ticks: Vec<_> = rig.sim.lock().haptics.drain(..).collect();
        assert_eq!(ticks.len(), 3, "one per face passed: {ticks:?}");
    }

    /// After a roll the screens stay as they were at the reveal, like a
    /// printed die: turning the die over to read it doesn't turn the text or
    /// move the dark face. A new shake lets them go (decision H9).
    #[test]
    fn a_result_keeps_its_orientation_until_the_next_throw() {
        let mut rig = Rig::new();
        rig.run(7.0);
        rig.world.set_next_landing(Face::PosZ);
        rig.world.start_shake();
        rig.run(0.5);
        rig.world.end_shake(true);
        rig.run(3.0);
        assert!(matches!(
            rig.fw.mode(),
            smokebomb_firmware::smokebomb_core::state::Mode::Reveal { .. }
        ));
        assert_eq!(rig.fw.display_up(), Face::PosZ);
        let frozen: Vec<_> = Face::ALL.iter().map(|&f| rig.fw.display_quarter(f)).collect();

        // Pick it up and set it down with +X on top.
        rig.world.place_face_up(Face::PosX);
        rig.run(1.5);
        let live: Vec<_> = Face::ALL
            .iter()
            .map(|&f| rig.fw.orientation().quarter(f))
            .collect();
        assert_ne!(live, frozen, "the die really did turn");
        assert_eq!(rig.fw.display_up(), Face::PosZ, "the dark face stays put");
        let shown: Vec<_> = Face::ALL.iter().map(|&f| rig.fw.display_quarter(f)).collect();
        assert_eq!(shown, frozen);

        // A new shake clears the result; shaken and set down, the die rolls
        // again, and the new result is frozen as the die now lies.
        rig.world.start_shake();
        rig.run(0.3);
        rig.world.end_shake(false);
        rig.run(2.0);
        assert!(matches!(
            rig.fw.mode(),
            smokebomb_firmware::smokebomb_core::state::Mode::Reveal { .. }
        ));
        assert_eq!(rig.fw.display_up(), Face::PosX);
        let shown: Vec<_> = Face::ALL.iter().map(|&f| rig.fw.display_quarter(f)).collect();
        let live: Vec<_> = Face::ALL
            .iter()
            .map(|&f| rig.fw.orientation().quarter(f))
            .collect();
        assert_eq!(shown, live);
    }

    /// The page is on the face in front of the viewer.
    fn assert_page_in_front(rig: &Rig, what: &str) {
        let toward_viewer = DEFAULT_VIEWER_RIGHT.cross(glam::Vec3::Y);
        let front = rig.fw.menu_front().expect("menu open");
        let n = rig.world.pose().rotation * smokebomb_hal_simulator::world::face_normal(front);
        assert!(
            n.dot(toward_viewer) > 0.9,
            "{what}: the page is on {front:?}, which isn't in front"
        );
    }

    fn firmware_dir(dir: TipDir) -> smokebomb_firmware::smokebomb_core::tips::TipDir {
        use smokebomb_firmware::smokebomb_core::tips::TipDir as F;
        match dir {
            TipDir::Up => F::Up,
            TipDir::Down => F::Down,
            TipDir::Left => F::Left,
            TipDir::Right => F::Right,
        }
    }

    impl Rig {
        /// Hold +Z until the menu opens, then let go.
        fn open_menu(&mut self) {
            self.sim.lock().touch_mask = 1 << Face::PosZ.index();
            while self.fw.menu_front().is_none() {
                self.run(DT);
            }
            self.sim.lock().touch_mask = 0;
        }

        /// Wait until the die can move again, then `extra` seconds.
        fn wait_then(&mut self, extra: f64) {
            while !self.world.twist(0.0) {
                self.run(DT);
            }
            self.run(0.35 + extra); // the zero twist, then the pause
        }
    }

    /// A real hand: tremor, a gyro bias, tips that are off-axis and too long
    /// or short, squared up afterwards to look at the page. Every tip still
    /// counts once and the page stays on the face in front.
    #[test]
    fn sloppy_hands_keep_the_page_in_front() {
        for seed in 0..SLOPPY_SEEDS {
            sloppy_hands(seed);
        }
    }

    const SLOPPY_SEEDS: u64 = 8;

    fn sloppy_hands(seed: u64) {
        let mut rig = Rig::with_world(World::with_seed(seed));
        rig.run(7.0);
        rig.world.set_hand(Hand {
            tremor_dps: 10.0,
            gyro_bias_dps: [1.5, -1.0, 0.8],
            tip_axis_error_deg: 15.0,
            tip_angle_error_deg: 15.0,
            resquare: true,
        });
        rig.open_menu();
        rig.wait_then(0.2);
        let mut expected = *rig.fw.menu_draft().unwrap();
        let dirs = [
            TipDir::Left,
            TipDir::Up,
            TipDir::Up,
            TipDir::Right,
            TipDir::Down,
            TipDir::Left,
            TipDir::Left,
            TipDir::Up,
            TipDir::Right,
            TipDir::Down,
            TipDir::Down,
            TipDir::Left,
            TipDir::Up,
            TipDir::Right,
            TipDir::Up,
            TipDir::Left,
        ];
        for (i, dir) in dirs.into_iter().enumerate() {
            assert!(rig.world.tip(dir, DEFAULT_VIEWER_RIGHT));
            expected = expected.tipped(firmware_dir(dir));
            rig.wait_then([0.1, 0.4, 0.2, 0.6][i % 4]);
            let what = format!("seed {seed}, tip {i} ({dir:?})");
            assert_page_in_front(&rig, &what);
            assert_eq!(*rig.fw.menu_draft().unwrap(), expected, "{what}");
        }
    }

    /// Twisting the die about the line of sight isn't a tip: the setup
    /// doesn't change, the page stays in front the right way up, and tips
    /// afterwards still land where they should.
    #[test]
    fn a_twist_is_not_a_tip() {
        use smokebomb_firmware::smokebomb_core::orientation;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.open_menu();
        rig.wait_then(0.2);
        let mut expected = *rig.fw.menu_draft().unwrap();
        for degrees in [30.0, -30.0, 90.0] {
            assert!(rig.world.twist(degrees));
            rig.wait_then(0.5);
            assert_page_in_front(&rig, &format!("twist {degrees}°"));
            assert_eq!(*rig.fw.menu_draft().unwrap(), expected, "twist {degrees}°");
        }
        // Rolled a quarter turn: the page is drawn upright for where the sky
        // now is.
        let front = rig.fw.menu_front().unwrap();
        let sky = rig.fw.orientation().quarter(front);
        assert_eq!(
            rig.fw.menu_page_quarter(front),
            Some(sky),
            "upright after the roll"
        );
        let _ = orientation::Quarter::R0;
        for dir in [TipDir::Left, TipDir::Up, TipDir::Right, TipDir::Down] {
            assert!(rig.world.tip(dir, DEFAULT_VIEWER_RIGHT));
            expected = expected.tipped(firmware_dir(dir));
            rig.wait_then(0.3);
            assert_page_in_front(&rig, &format!("{dir:?} after the roll"));
            assert_eq!(*rig.fw.menu_draft().unwrap(), expected, "{dir:?} after the roll");
        }
    }

    /// Tips in quick succession, each starting the moment the last one
    /// ends (and the first straight after the menu turns to the viewer):
    /// every one counts, and the page is always on the face in front.
    #[test]
    fn quick_tips_in_a_row_keep_the_page_in_front() {
        let toward_viewer = DEFAULT_VIEWER_RIGHT.cross(glam::Vec3::Y);
        let dirs = [
            TipDir::Left,
            TipDir::Up,
            TipDir::Left,
            TipDir::Down,
            TipDir::Right,
            TipDir::Up,
            TipDir::Up,
            TipDir::Left,
            TipDir::Down,
            TipDir::Right,
        ];
        let mut rig = Rig::new();
        rig.run(7.0);
        // Hold +Z, and tip as soon as the die can move after the snap.
        rig.sim.lock().touch_mask = 1 << Face::PosZ.index();
        while rig.fw.menu_front().is_none() {
            rig.run(DT);
        }
        rig.sim.lock().touch_mask = 0;
        let mut expected = *rig.fw.menu_draft().unwrap();
        for dir in dirs {
            while !rig.world.tip(dir, DEFAULT_VIEWER_RIGHT) {
                rig.run(DT);
            }
            expected = expected.tipped(match dir {
                TipDir::Up => smokebomb_firmware::smokebomb_core::tips::TipDir::Up,
                TipDir::Down => smokebomb_firmware::smokebomb_core::tips::TipDir::Down,
                TipDir::Left => smokebomb_firmware::smokebomb_core::tips::TipDir::Left,
                TipDir::Right => smokebomb_firmware::smokebomb_core::tips::TipDir::Right,
            });
        }
        rig.run(1.0);
        let front = rig.fw.menu_front().unwrap();
        let n = rig.world.pose().rotation * smokebomb_hal_simulator::world::face_normal(front);
        assert!(
            n.dot(toward_viewer) > 0.99,
            "the page is on {front:?}, which isn't in front"
        );
        assert_eq!(*rig.fw.menu_draft().unwrap(), expected);
    }

    /// The screen coming round in a tip reads the right way up from the
    /// start: its orientation doesn't change during the tip, and matches the
    /// die's own orientation for that face once it's in front.
    #[test]
    fn a_tip_brings_the_next_screen_round_the_right_way_up() {
        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        for dir in [
            TipDir::Up,
            TipDir::Up,
            TipDir::Down,
            TipDir::Left,
            TipDir::Down,
            TipDir::Right,
            TipDir::Up,
        ] {
            assert!(rig.world.tip(dir, DEFAULT_VIEWER_RIGHT));
            let mut seen = Vec::new();
            for _ in 0..36 {
                rig.run(DT);
                for face in Face::ALL {
                    if let Some(q) = rig.fw.menu_page_quarter(face) {
                        seen.push((face, q));
                    }
                }
            }
            rig.run(0.5);
            let front = rig.fw.menu_front().unwrap();
            let settled = rig.fw.orientation().quarter(front);
            assert_eq!(rig.fw.menu_page_quarter(front), Some(settled), "{dir:?}: settled");
            for (face, q) in seen {
                if face == front {
                    assert_eq!(
                        q, settled,
                        "{dir:?}: {front:?} came round as {q:?}, reads {settled:?}"
                    );
                }
            }
        }
    }

    /// Many tips in a row, at the server's real tick: every one counts once,
    /// and the firmware's front face stays the one facing the viewer.
    #[test]
    fn a_long_run_of_tips_stays_in_step() {
        use smokebomb_firmware::smokebomb_core::menu::Page;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        for _ in 0..13 {
            rig.tip(TipDir::Left);
            rig.run(0.3);
        }
        // 13 pages on, 4 pages to a cycle: one on from How many dice.
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Die);
        for _ in 0..7 {
            rig.tip(TipDir::Up);
            rig.run(0.3);
        }
        // Seven values up from d20 is a lap of the seven dice: d100, d4, d6, d8, d10, d12, d20.
        assert_eq!(rig.fw.menu_draft().unwrap().die, smokebomb_shared::DieKind::D20);
        let front = rig.fw.menu_front().unwrap();
        let toward_viewer = DEFAULT_VIEWER_RIGHT.cross(glam::Vec3::Y);
        let n = rig.world.pose().rotation * smokebomb_hal_simulator::world::face_normal(front);
        assert!(n.dot(toward_viewer) > 0.99, "{front:?} is not in front");
    }

    /// Once a tip has stopped, the screens already show the settled menu:
    /// nothing moves or snaps when the turn is counted a moment later.
    #[test]
    fn a_finished_tip_shows_centred_before_it_settles() {
        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        for dir in [TipDir::Left, TipDir::Up, TipDir::Right, TipDir::Down] {
            assert!(rig.world.tip(dir, DEFAULT_VIEWER_RIGHT));
            rig.run(0.5); // the turn is over; the tracker is still settling
            let during = rig.sim.lock().faces;
            rig.run(0.5);
            let after = rig.sim.lock().faces;
            assert!(during == after, "{dir:?}: screens changed when the tip settled");
        }
    }

    #[test]
    fn a_multi_turn_moves_as_many_steps_as_faces() {
        use smokebomb_firmware::smokebomb_core::menu::Page;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        // Two faces left: two pages on (How many dice → Settings).
        rig.spin(SpinAxis::Yaw, -2.2);
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Settings);
        // Back one and a bit: settles one face back (Which die).
        rig.spin(SpinAxis::Yaw, 1.3);
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Die);
        // Three faces up: d20 → d100 → d4 → d6.
        rig.spin(SpinAxis::Pitch, -3.0);
        let draft = rig.fw.menu_draft().unwrap();
        assert_eq!(draft.die, smokebomb_shared::DieKind::D6);
        // The firmware's front face is the one really facing the viewer.
        let front = rig.fw.menu_front().unwrap();
        let toward_viewer = DEFAULT_VIEWER_RIGHT.cross(glam::Vec3::Y);
        let n = rig.world.pose().rotation * smokebomb_hal_simulator::world::face_normal(front);
        assert!(n.dot(toward_viewer) > 0.99, "{front:?} is not in front");
    }

    #[test]
    fn menu_tips_change_the_setup_and_a_hold_saves_it() {
        use smokebomb_firmware::smokebomb_core::menu::Page;
        use smokebomb_firmware::smokebomb_core::state::Mode;
        use smokebomb_shared::DieKind;

        let mut rig = Rig::new();
        rig.run(7.0); // boot
                      // Held on a side face that isn't facing the viewer: the die turns it
                      // round first.
        rig.hold(Face::NegX);
        assert_eq!(*rig.fw.mode(), Mode::Menu);
        assert_eq!(rig.fw.menu_front(), Some(Face::NegX));
        rig.tip(TipDir::Up);
        assert_eq!(rig.fw.menu_draft().unwrap().count, 2);
        rig.tip(TipDir::Left);
        rig.tip(TipDir::Down);
        let draft = *rig.fw.menu_draft().unwrap();
        assert_eq!((draft.page, draft.die), (Page::Die, DieKind::D12));
        assert_eq!(rig.fw.settings().die, DieKind::D20, "nothing changes until saved");
        rig.hold(Face::PosY);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert_eq!(
            (rig.fw.settings().die, rig.fw.settings().count),
            (DieKind::D12, 2)
        );
    }

    #[test]
    fn the_mode_page_switches_to_pass_the_pot_and_back_keeping_the_dice() {
        use smokebomb_firmware::smokebomb_core::menu::{Page, PlayMode};
        use smokebomb_shared::DieKind;

        let mut rig = Rig::new();
        rig.run(7.0);
        // Set up 2d12 first.
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Up);
        rig.tip(TipDir::Left);
        rig.tip(TipDir::Down);
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.settings().active(), (DieKind::D12, 2));

        // Mode is one tip right of the count.
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Count);
        rig.tip(TipDir::Right);
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Mode);
        rig.tip(TipDir::Up);
        assert_eq!(rig.fw.menu_draft().unwrap().play, PlayMode::PassThePot);
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.settings().active(), (DieKind::PassThePot, 1));

        // And back: the dice setup is still 2d12.
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Pot);
        rig.tip(TipDir::Right);
        rig.tip(TipDir::Down);
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.settings().active(), (DieKind::D12, 2));
    }

    #[test]
    fn hot_potato_lights_on_a_shake_goes_off_and_resets_without_rolling() {
        use smokebomb_firmware::smokebomb_core::menu::PlayMode;
        use smokebomb_firmware::smokebomb_core::potato::{PotatoState, BOOM_MS};
        use smokebomb_firmware::smokebomb_core::state::Mode;
        use smokebomb_hal::HapticEffect;

        let mut rig = Rig::new();
        rig.run(7.0);
        // Mode ▶ Hot Potato, then Fuse length ▶ Short, and save.
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Right);
        rig.tip(TipDir::Up);
        rig.tip(TipDir::Up);
        rig.tip(TipDir::Left);
        rig.tip(TipDir::Down);
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.settings().play(), PlayMode::HotPotato);
        assert!(rig.fw.potato().is_idle());
        rig.sim.lock().haptics.clear();

        // A shake lights the fuse and the die doesn't roll.
        rig.world.start_shake();
        rig.run(1.0);
        assert!(rig.fw.potato().is_lit(), "{:?}", rig.fw.potato().state());
        rig.world.end_shake(false);
        rig.run(3.0);
        assert!(rig.fw.potato().is_lit());
        assert_eq!(*rig.fw.mode(), Mode::Idle, "the roll flow stays out of it");
        assert!(rig.fw.last_roll().is_none());
        let PotatoState::Lit { fuse_ms, .. } = *rig.fw.potato().state() else {
            panic!("not lit");
        };
        assert!((10_000..=20_000).contains(&fuse_ms), "short fuse: {fuse_ms}");

        // Holding doesn't open the menu mid-round.
        rig.hold(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(rig.fw.menu_draft().is_none());

        // It goes off within the fuse.
        for _ in 0..(25.0 / 0.1) as usize {
            if matches!(rig.fw.potato().state(), PotatoState::Boom { .. }) {
                break;
            }
            rig.run(0.1);
        }
        assert!(matches!(rig.fw.potato().state(), PotatoState::Boom { .. }));
        let haptics: Vec<_> = rig.sim.lock().haptics.iter().copied().collect();
        assert!(haptics.iter().filter(|h| **h == HapticEffect::Tick).count() > 10);
        assert!(haptics.contains(&HapticEffect::Buzz));
        assert!(rig.fw.last_roll().is_none());

        // A tap resets it once BOOM has had its moment.
        rig.run(1.0);
        rig.sim.lock().touch_mask = 1 << Face::PosZ.index();
        rig.run(0.1);
        rig.sim.lock().touch_mask = 0;
        rig.run(0.1);
        assert!(rig.fw.potato().is_idle());

        // Or BOOM times out by itself.
        rig.world.start_shake();
        rig.run(1.0);
        rig.world.end_shake(false);
        for _ in 0..(25.0 / 0.1) as usize {
            if matches!(rig.fw.potato().state(), PotatoState::Boom { .. }) {
                break;
            }
            rig.run(0.1);
        }
        assert!(matches!(rig.fw.potato().state(), PotatoState::Boom { .. }));
        rig.run(BOOM_MS as f64 / 1000.0 + 0.5);
        assert!(rig.fw.potato().is_idle());
    }

    #[test]
    fn tapping_power_off_darkens_the_die_and_a_tap_boots_it() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Up); // 2 dice, not saved
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings
        rig.tip(TipDir::Down); // About
        rig.tip(TipDir::Down); // Power off
        rig.tap(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Off);
        assert!(!rig.sim.lock().display_on);
        assert_eq!(rig.fw.settings().count, 1, "not saved");
        // Stays dark, ignoring a long wait.
        rig.run(30.0);
        assert_eq!(*rig.fw.mode(), Mode::Off);
        assert!(rig.sim.lock().faces.iter().all(|f| f.iter().all(|b| *b == 0)));
        // A tap boots straight away.
        rig.tap(Face::PosY);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(rig.sim.lock().display_on);
        assert!(rig.fw.booting());
    }

    #[test]
    fn tap_changes_a_setting_and_a_hold_saves_and_returns_to_the_roll() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings, on Brightness (70%)
        rig.tap(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Menu, "a tap doesn't leave the menu");
        assert_eq!(rig.fw.menu_draft().unwrap().setting().1, "100%");
        assert_eq!(rig.fw.settings().brightness_pct(), 70, "not saved yet");
        rig.hold(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert_eq!(rig.fw.settings().brightness_pct(), 100);
    }

    #[test]
    fn a_powered_off_die_does_not_light_hot_potato() {
        use smokebomb_firmware::smokebomb_core::menu::PlayMode;
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        // Mode ▶ Hot Potato, and save.
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Right);
        rig.tip(TipDir::Up);
        rig.tip(TipDir::Up);
        rig.hold(Face::PosZ);
        assert_eq!(rig.fw.settings().play(), PlayMode::HotPotato);
        // Fuse length ▶ Settings ▶ Power off, and tap.
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Left);
        rig.tip(TipDir::Down);
        rig.tip(TipDir::Down);
        rig.tap(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Off);

        // A shake doesn't light the fuse, and a tap wakes the die instead of
        // passing the potato.
        rig.world.start_shake();
        rig.run(1.0);
        rig.world.end_shake(false);
        rig.run(2.0);
        assert_eq!(*rig.fw.mode(), Mode::Off);
        assert!(rig.fw.potato().is_idle());
        rig.tap(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(rig.fw.potato().is_idle());
    }

    /// The default settings with some items chosen (by index into
    /// `SETTINGS`: 2 is Smoke, 3 is Sleep after).
    fn settings_with(choices: &[(usize, u8)]) -> smokebomb_firmware::smokebomb_core::menu::Settings {
        let mut s = smokebomb_firmware::smokebomb_core::menu::Settings::default();
        for &(item, option) in choices {
            s.choices[item] = option;
        }
        s
    }

    fn lit(rig: &Rig) -> bool {
        rig.sim.lock().faces.iter().any(|f| f.iter().any(|b| *b != 0))
    }

    #[test]
    fn about_shows_the_dies_own_id_and_keeps_it_when_settings_are_replaced() {
        use smokebomb_firmware::smokebomb_core::menu::short_id;

        let mut rig = Rig::new();
        let id = short_id(&smokebomb_hal_simulator::SIM_SERIAL);
        assert_ne!(id, 0);
        assert_eq!(rig.fw.settings().device_id, id);
        rig.fw.set_settings(settings_with(&[(2, 1)]));
        assert_eq!(rig.fw.settings().device_id, id, "loading settings keeps the id");
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings, on Brightness
        rig.tip(TipDir::Down); // About
        let draft = rig.fw.menu_draft().unwrap();
        assert_eq!(draft.setting().0, "About");
        assert!(draft.setting_value().ends_with(&format!("SB-{id:04X}")));
    }

    #[test]
    fn the_screens_sleep_after_the_chosen_idle_time_and_wake_on_a_tap() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.fw.set_settings(settings_with(&[(3, 0)])); // Sleep after 30 s
        rig.run(7.0); // boot
        rig.tap(Face::PosZ);
        assert!(lit(&rig), "the tap shows the setup");
        rig.run(20.0);
        assert!(rig.sim.lock().display_on, "still awake at 20 s");
        rig.run(15.0);
        assert!(!rig.sim.lock().display_on, "asleep after 30 s");
        assert!(!lit(&rig));
        assert_eq!(*rig.fw.mode(), Mode::Idle, "sleeping isn't a mode");

        // A tap wakes it with the setup label, and no boot.
        rig.tap(Face::PosY);
        assert!(rig.sim.lock().display_on);
        assert!(lit(&rig));
        assert!(!rig.fw.booting());
    }

    #[test]
    fn touching_the_die_puts_off_sleep() {
        let mut rig = Rig::new();
        rig.fw.set_settings(settings_with(&[(3, 0)]));
        rig.run(7.0);
        rig.run(25.0);
        rig.tap(Face::PosZ);
        rig.run(25.0);
        assert!(rig.sim.lock().display_on, "50 s in but 25 s since the tap");
        rig.run(10.0);
        assert!(!rig.sim.lock().display_on);
    }

    #[test]
    fn a_sleeping_die_still_rolls_when_thrown() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.fw.set_settings(settings_with(&[(3, 0)]));
        rig.run(45.0);
        assert!(!rig.sim.lock().display_on);
        rig.world.start_shake();
        rig.run(1.0);
        assert!(rig.sim.lock().display_on, "a shake wakes it");
        rig.world.end_shake(true);
        for _ in 0..(10.0 / 0.1) as usize {
            if matches!(rig.fw.mode(), Mode::Reveal { .. }) {
                break;
            }
            rig.run(0.1);
        }
        assert!(matches!(rig.fw.mode(), Mode::Reveal { .. }));
        assert!(rig.fw.last_roll().is_some());
    }

    #[test]
    fn sleep_never_stays_awake() {
        let mut rig = Rig::new();
        rig.fw.set_settings(settings_with(&[(3, 4)])); // Never
        rig.run(7.0);
        assert_eq!(rig.fw.settings().sleep_after_ms(), None);
        rig.run(200.0);
        assert!(rig.sim.lock().display_on);
    }

    #[test]
    fn smoke_off_makes_no_smoke_and_light_makes_less() {
        let cloud = |choice: Option<u8>| {
            let mut rig = Rig::new();
            if let Some(c) = choice {
                rig.fw.set_settings(settings_with(&[(2, c)]));
            }
            rig.run(7.0);
            rig.world.start_shake();
            rig.run(1.5);
            rig.fw.smoke_mut().len()
        };
        let (full, light, off) = (cloud(None), cloud(Some(1)), cloud(Some(0)));
        assert!(full > 0, "a shake builds smoke");
        assert!(light > 0 && light < full, "light {light} < full {full}");
        assert_eq!(off, 0);
    }

    #[test]
    fn smoke_chosen_in_the_menu_applies_when_saved() {
        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings, on Brightness
        rig.tip(TipDir::Up); // Haptics
        rig.tip(TipDir::Up); // Smoke
        rig.tap(Face::PosZ); // Full -> Off
        assert_eq!(rig.fw.menu_draft().unwrap().setting(), ("Smoke", "Off"));
        rig.hold(Face::PosZ);
        rig.world.start_shake();
        rig.run(1.5);
        assert_eq!(rig.fw.smoke_mut().len(), 0, "no smoke once Off is saved");
    }

    #[test]
    fn a_hold_on_power_off_saves_and_returns_like_anywhere_else() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Up); // 2 dice
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings
        rig.tip(TipDir::Down); // About
        rig.tip(TipDir::Down); // Power off
        rig.hold(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(!rig.fw.booting(), "still on");
        assert_eq!(rig.fw.settings().count, 2, "saved");
    }
}
