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
            die: firmware.settings().die.wire_name(),
            die_count: firmware.settings().count,
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
        let s = fw.settings();
        status.die = s.die.wire_name();
        status.die_count = s.count;
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
    use smokebomb_hal_simulator::world::{SpinAxis, TipDir, DEFAULT_VIEWER_RIGHT};

    impl Rig {
        fn new() -> Self {
            let sim = board::SimHandle::new();
            sim.lock().manual_time_ms = Some(0);
            let fw = board::boot(&sim).unwrap();
            Self {
                sim,
                fw,
                world: World::new(),
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
        // 13 pages on, 3 pages to a cycle: one on from How many dice.
        assert_eq!(rig.fw.menu_draft().unwrap().page, Page::Die);
        for _ in 0..7 {
            rig.tip(TipDir::Up);
            rig.run(0.3);
        }
        // Seven values up from d20, wrapping: d100, Pot, d4, d6, d8, d10, d12.
        assert_eq!(rig.fw.menu_draft().unwrap().die, smokebomb_shared::DieKind::D12);
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
        // Three faces up: d20 → d100 → Pass the Pot → d4.
        rig.spin(SpinAxis::Pitch, -3.0);
        let draft = rig.fw.menu_draft().unwrap();
        assert_eq!(draft.die, smokebomb_shared::DieKind::D4);
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
    fn holding_on_restart_restarts_without_saving() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Up); // 2 dice, not saved
        rig.tip(TipDir::Right); // Settings
        rig.tip(TipDir::Down); // About
        rig.tip(TipDir::Down); // Restart
        rig.hold(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(rig.fw.booting());
        assert_eq!(rig.fw.settings().count, 1);
    }
}
