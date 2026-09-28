use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use smokebomb_hal::Face;
use tokio::sync::broadcast::error::RecvError;

use crate::protocol::{Inbound, Outbound};
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
                Ok(Outbound::Binary(f)) => {
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
    let mut world = state.world.lock().unwrap_or_else(|e| e.into_inner());
    match input {
        Inbound::Touch { face, pressed } => {
            if let Some(f) = Face::from_index(face as usize) {
                let bit = 1u8 << f.index();
                let mut s = state.sim.lock();
                s.touch_mask = if pressed {
                    s.touch_mask | bit
                } else {
                    s.touch_mask & !bit
                };
            }
        }
        Inbound::ShakeStart => world.start_shake(),
        Inbound::ShakeEnd { throw } => world.end_shake(throw),
        Inbound::Tip { dir, right } => {
            world.tip(dir.into(), glam::Vec3::from_array(right));
        }
        Inbound::Rotate { yaw, pitch } => world.rotate(yaw, pitch),
        Inbound::PlaceFaceUp { face } => {
            if let Some(f) = Face::from_index(face as usize) {
                world.place_face_up(f);
            }
        }
        Inbound::Dock { docked } => world.set_docked(docked),
        Inbound::Ble { connected } => state.sim.lock().ble_connected = connected,
        Inbound::ReducedMotion { on } => world.set_reduced_motion(on),
    }
}
