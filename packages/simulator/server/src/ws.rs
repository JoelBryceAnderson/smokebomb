use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use smokebomb_hal::{Face, ImuSample};
use smokebomb_hal_simulator::imu_script;
use tokio::sync::broadcast::error::RecvError;

use crate::protocol::{Gesture, Inbound, Outbound};
use crate::AppState;

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| session(socket, state))
}

async fn session(mut socket: WebSocket, state: AppState) {
    let mut rx = state.out.subscribe();
    // Force a full frame push to the new client on the next tick.
    state.sim.lock().frame_seq += 1;

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(Outbound::Frames(f)) => {
                    if socket.send(Message::Binary(f.as_ref().clone().into())).await.is_err() { break; }
                }
                Ok(Outbound::Event(e)) => {
                    if socket.send(Message::Text(e.as_str().into())).await.is_err() { break; }
                }
                Err(RecvError::Lagged(n)) => tracing::debug!("ws client lagged by {n} messages"),
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Inbound>(&text) {
                    Ok(input) => apply(&state, input),
                    Err(e) => tracing::warn!("bad input {text:?}: {e}"),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}

fn apply(state: &AppState, input: Inbound) {
    let mut s = state.sim.lock();
    match input {
        Inbound::Touch { face, pressed } => {
            if let Some(f) = Face::from_index(face as usize) {
                let bit = 1u8 << f.index();
                s.touch_mask = if pressed {
                    s.touch_mask | bit
                } else {
                    s.touch_mask & !bit
                };
            }
        }
        Inbound::Orient { up } => {
            if let Some(f) = Face::from_index(up as usize) {
                s.imu_resting = imu_script::resting(f);
            }
        }
        Inbound::Imu { accel, gyro } => {
            s.imu_resting = ImuSample {
                accel_mg: accel,
                gyro_mdps: gyro,
            };
        }
        Inbound::Gesture { kind, land } => {
            let samples = match kind {
                Gesture::PickUp => imu_script::pick_up(),
                Gesture::Shake => imu_script::shake(),
                Gesture::Throw => imu_script::throw(),
            };
            s.imu_script.extend(samples);
            if matches!(kind, Gesture::Throw) {
                let face = land
                    .and_then(|l| Face::from_index(l as usize))
                    .unwrap_or_else(random_face);
                s.imu_resting = imu_script::resting(face);
            }
        }
        Inbound::Dock { docked } => s.docked = docked,
        Inbound::Ble { connected } => s.ble_connected = connected,
    }
}

fn random_face() -> Face {
    let mut b = [0u8; 1];
    let _ = getrandom::getrandom(&mut b);
    Face::ALL[b[0] as usize % 6]
}
