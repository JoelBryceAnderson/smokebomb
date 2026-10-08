//! The browser simulator: an emulator thread at the Game Boy's frame rate,
//! and a WebSocket to the page.
//!
//! Browser → server (JSON):
//! * `{"type":"imu","accel":[x,y,z]}`: the accelerometer in milli-g, die
//!   axes (the page owns the cube's pose and sends what the sensor would
//!   read).
//! * `{"type":"touch","mask":n}`: bit n = face n touched.
//! * `{"type":"keys","buttons":n}`: joypad buttons held on the keyboard.
//! * `{"type":"config", ...}`: any of `wrap`, `ui`, `fog`, `deadzone`,
//!   `walk_max_deg`, `walk_hold_ms`, `switch_deg`, `long_press_ms`,
//!   `tap_max_ms`, `shake_mg`, `debug`.
//! * `{"type":"reset"}`.
//! * `{"type":"ready"}`: send the next frame. Frames are pulled, one at a
//!   time, so a slow page sees the latest frame instead of a growing
//!   backlog (and its input isn't stuck behind seconds of video).
//!
//! Server → browser:
//! * binary, every frame: `GCB1`, frame number (u32 LE), flags (bit 0: a
//!   prediction follows), 3 reserved bytes, then six 64×64 faces, the
//!   160×144 frame and (flag 0) the world renderer's prediction of that
//!   frame, all RGB565 little endian.
//! * JSON `status` a few times a second.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::cube::{Config, Wrap};
use gbc_cube_core::fallback::UiStyle;
use gbc_cube_core::geom::Compass;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeader;

use crate::session::{Cart, Input, Session};

#[derive(Clone)]
struct AppState {
    input: Arc<Mutex<Shared>>,
    frames: broadcast::Sender<Arc<Vec<u8>>>,
    status: broadcast::Sender<Arc<String>>,
}

/// What the page has told us, read by the emulator thread every frame.
#[derive(Default)]
struct Shared {
    input: Input,
    cfg: Config,
    /// `cfg` changed since the emulator thread last took it.
    cfg_changed: bool,
    debug: bool,
    reset: bool,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FromPage {
    Imu { accel: [f32; 3] },
    Touch { mask: u8 },
    Keys { buttons: u8 },
    Config(ConfigMsg),
    Reset,
    Ready,
}

#[derive(Deserialize, Default)]
struct ConfigMsg {
    wrap: Option<String>,
    ui: Option<String>,
    fog: Option<u8>,
    deadzone: Option<f32>,
    walk_max_deg: Option<f32>,
    walk_hold_ms: Option<u32>,
    switch_deg: Option<f32>,
    long_press_ms: Option<u32>,
    tap_max_ms: Option<u32>,
    shake_mg: Option<u32>,
    debug: Option<bool>,
}

pub async fn serve(cart: Cart, port: u16, web_dir: PathBuf) -> Result<()> {
    let (frames, _) = broadcast::channel(4);
    let (status, _) = broadcast::channel(16);
    let state = AppState {
        input: Arc::new(Mutex::new(Shared::default())),
        frames,
        status,
    };
    // Fail now, not in the thread, if the ROM won't load.
    let session = Session::open(&cart, Config::default())?;
    tracing::info!(
        "running {:?} ({})",
        session.title,
        if session.crystal {
            "Crystal RAM layout"
        } else {
            "frame only"
        }
    );
    if let Some(p) = session.emu.save_path() {
        tracing::info!("battery save: {}", p.display());
    }
    {
        let state = state.clone();
        std::thread::Builder::new()
            .name("emulator".into())
            .spawn(move || run(session, cart, state))?;
    }

    if !web_dir.join("index.html").exists() {
        tracing::warn!(
            "no web UI at {}: build it with `npm install && npm run build` in experiments/gbc-cube/web",
            web_dir.display()
        );
    }
    let files = SetResponseHeader::overriding(
        ServeDir::new(&web_dir).fallback(ServeFile::new(web_dir.join("index.html"))),
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    let app = Router::new()
        .route("/ws", get(ws))
        .fallback_service(files)
        .with_state(state);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("open http://localhost:{port}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn apply(cfg: &mut Config, m: &ConfigMsg) {
    match m.wrap.as_deref() {
        Some("frame") => cfg.wrap = Wrap::Frame,
        Some("world") => cfg.wrap = Wrap::World,
        _ => {}
    }
    match m.ui.as_deref() {
        Some("pan") => cfg.ui = UiStyle::Pan,
        Some("spread") => cfg.ui = UiStyle::Spread,
        Some("front") => cfg.ui = UiStyle::Front,
        _ => {}
    }
    if let Some(v) = m.fog {
        cfg.fog_px = v;
    }
    if let Some(v) = m.deadzone {
        cfg.controls.deadzone_deg = v;
    }
    if let Some(v) = m.walk_max_deg {
        cfg.controls.walk_max_deg = v;
    }
    if let Some(v) = m.walk_hold_ms {
        cfg.controls.walk_hold_ms = v;
    }
    if let Some(v) = m.switch_deg {
        cfg.up.switch_deg = v;
    }
    if let Some(v) = m.long_press_ms {
        cfg.controls.long_press_ms = v;
    }
    if let Some(v) = m.tap_max_ms {
        cfg.controls.tap_max_ms = v;
    }
    if let Some(v) = m.shake_mg {
        cfg.controls.shake_mg = v;
    }
}

fn face_name(i: usize) -> &'static str {
    ["+X", "-X", "+Y", "-Y", "+Z", "-Z"][i]
}

/// The emulator thread: one frame per 1/59.73 s, paced against the clock.
fn run(mut session: Session, cart: Cart, state: AppState) {
    let period = Duration::from_secs_f64(1.0 / gbc_cube_emu::FRAME_HZ);
    let mut next = Instant::now();
    let mut stats_window = (Instant::now(), 0u32, 0u64, 0u64);
    let mut fps = 0.0f32;
    let mut packet = Vec::new();
    loop {
        let (input, cfg, debug, reset) = {
            let mut s = state.input.lock().unwrap();
            let cfg = std::mem::take(&mut s.cfg_changed).then_some(s.cfg);
            (s.input, cfg, s.debug, std::mem::take(&mut s.reset))
        };
        if reset {
            match Session::open(&cart, session.cube.cfg) {
                Ok(s) => session = s,
                Err(e) => tracing::error!("reset failed: {e:#}"),
            }
        }
        if let Some(c) = cfg {
            session.cube.cfg = c;
        }
        let frame_no = session.frames + 1;
        let out = match session.tick(&input) {
            Ok(o) => o,
            Err(e) => {
                tracing::error!("emulator stopped: {e:#}");
                return;
            }
        };
        packet.clear();
        packet.extend_from_slice(b"GCB1");
        packet.extend_from_slice(&(frame_no as u32).to_le_bytes());
        let with_prediction = debug && out.predicted.is_some();
        packet.extend_from_slice(&[u8::from(with_prediction), 0, 0, 0]);
        for face in out.faces.iter() {
            for p in face {
                packet.extend_from_slice(&p.to_le_bytes());
            }
        }
        for p in out.frame.iter() {
            packet.extend_from_slice(&p.to_le_bytes());
        }
        if with_prediction {
            for p in out.predicted.unwrap() {
                packet.extend_from_slice(&p.to_le_bytes());
            }
        }
        let _ = state.frames.send(Arc::new(packet.clone()));

        stats_window.1 += 1;
        stats_window.2 += out.emu_us as u64;
        stats_window.3 += out.render_us as u64;
        let r = out.report;
        let status = (stats_window.1 % 10 == 0).then(|| {
            let h = r.heading;
            let side = |c: Compass| face_name(h.side_face(c).index());
            json!({
                "type": "status",
                "scene": format!("{:?}", r.scene),
                "drawn": format!("{:?}", r.drawn),
                "up": face_name(h.up_face().index()),
                "north": side(Compass::North),
                "east": side(Compass::East),
                "south": side(Compass::South),
                "west": side(Compass::West),
                "buttons": out.buttons.bits(),
                "mismatches": out.mismatches,
                "reused": r.reused,
                "needs_frame": r.needs_frame(),
                "ui_tiles": r.info.ui_tiles,
                "stray_tiles": r.info.stray_tiles,
                "camera": r.camera.map(|c| [c.screen.0, c.screen.1]),
                "centre": r.camera.map(|c| [c.centre.0, c.centre.1]),
            })
        });
        let walking = session.cube.walking().map(|c| format!("{c:?}"));
        let rolls = session.cube.rolls();
        let frames = session.frames;
        if let Some(mut s) = status {
            let since = stats_window.0.elapsed().as_secs_f32();
            if since >= 1.0 {
                fps = stats_window.1 as f32 / since;
                let rom = session.take_rom_stats();
                let n = stats_window.1.max(1) as u64;
                s["emu_us"] = json!(stats_window.2 / n);
                s["render_us"] = json!(stats_window.3 / n);
                s["bank_changes_per_frame"] = json!(rom.bank_changes / rom.frames.max(1));
                s["banks_touched"] = json!(rom.banks_touched);
                stats_window = (Instant::now(), 0, 0, 0);
            }
            s["fps"] = json!(fps);
            s["walking"] = json!(walking);
            s["rolls"] = json!(rolls);
            s["frame"] = json!(frames);
            s["title"] = json!(session.title);
            s["crystal"] = json!(session.crystal);
            s["demo"] = json!(session.demo.is_some());
            let _ = state.status.send(Arc::new(s.to_string()));
        }

        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(100) {
            next = now; // fell behind (a breakpoint, a slow machine): resync
        }
    }
}

async fn ws(upgrade: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket| client(socket, state))
}

async fn client(mut socket: WebSocket, state: AppState) {
    let mut frames = state.frames.subscribe();
    let mut status = state.status.subscribe();
    let mut wants_frame = true;
    loop {
        tokio::select! {
            f = frames.recv() => match f {
                Ok(p) => {
                    if !wants_frame {
                        continue;
                    }
                    wants_frame = false;
                    if socket.send(Message::Binary(p.as_ref().clone().into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            },
            s = status.recv() => if let Ok(s) = s {
                if socket.send(Message::Text(s.as_ref().clone().into())).await.is_err() {
                    break;
                }
            },
            m = socket.recv() => match m {
                Some(Ok(Message::Text(t))) if t.as_str() == r#"{"type":"ready"}"# => wants_frame = true,
                Some(Ok(Message::Text(t))) => handle(&state, &t),
                Some(Ok(_)) => {}
                _ => break,
            },
        }
    }
}

fn handle(state: &AppState, text: &str) {
    let Ok(msg) = serde_json::from_str::<FromPage>(text) else {
        tracing::debug!("ignoring {text}");
        return;
    };
    let mut s = state.input.lock().unwrap();
    match msg {
        FromPage::Imu { accel } => s.input.imu.accel_mg = accel.map(|a| a.clamp(-16000.0, 16000.0) as i16),
        FromPage::Touch { mask } => s.input.touch = mask,
        FromPage::Keys { buttons } => s.input.keys = Buttons::from_bits(buttons),
        FromPage::Config(m) => {
            if let Some(d) = m.debug {
                s.debug = d;
            }
            apply(&mut s.cfg, &m);
            s.cfg_changed = true;
        }
        FromPage::Reset => s.reset = true,
        FromPage::Ready => {}
    }
}
