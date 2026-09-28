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
mod world;
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

        // "Reveal { since_ms: 123 }" -> "Reveal"; keeps "Menu(DieType)" intact.
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
