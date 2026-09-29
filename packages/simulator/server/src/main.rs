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
use axum::routing::get;
use axum::{Json, Router};
use smokebomb_firmware::board::{self, SimHandle};
use smokebomb_hal::SecureElement;
use smokebomb_hal_simulator::SimSecureElement;
use tokio::sync::{broadcast, Mutex};
use tower_http::services::{ServeDir, ServeFile};

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
    let static_files = ServeDir::new(&web_dir).fallback(ServeFile::new(web_dir.join("index.html")));

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

/// Firmware main loop: tick, then publish anything that changed.
async fn run_firmware(mut fw: board::Firmware, state: AppState) {
    let period = Duration::from_millis(1000 / smokebomb_firmware::smokebomb_core::TICK_HZ as u64);
    let mut interval = tokio::time::interval(period);
    let mut last_seq = 0;
    let mut last_mode = String::new();
    let mut last_counter = None;
    let mut last_pose = None;
    let mut menu_was_open = false;
    let dt = period.as_secs_f64();

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

    const DT: f64 = 1.0 / 60.0;

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
    use smokebomb_hal_simulator::world::{TipDir, DEFAULT_VIEWER_RIGHT};

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
    fn holding_on_restart_restarts_without_saving() {
        use smokebomb_firmware::smokebomb_core::state::Mode;

        let mut rig = Rig::new();
        rig.run(7.0);
        rig.hold(Face::PosZ);
        rig.tip(TipDir::Up); // 2 dice, not saved
        rig.tip(TipDir::Left); // Die
        rig.tip(TipDir::Left); // Settings
        rig.tip(TipDir::Down); // About
        rig.tip(TipDir::Down); // Restart
        rig.hold(Face::PosZ);
        assert_eq!(*rig.fw.mode(), Mode::Idle);
        assert!(rig.fw.booting());
        assert_eq!(rig.fw.settings().count, 1);
    }
}
