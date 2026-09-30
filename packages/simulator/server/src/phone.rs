//! The phone link: the app's BLE messages over a WebSocket.
//!
//! On hardware the phone talks to the die over BLE with the
//! `smokebomb_shared::protocol` messages. Here the same messages travel as
//! JSON text frames on `/phone`, one message per frame, in serde's default
//! shape: `"GetInventory"`, `{"SetEnabledModes":13}`,
//! `{"Inventory":{"licensed":15,"enabled":13,"active":"Dice"}}`. The app's
//! simulator link (`SimulatorBleManager`) connects here, so an iPhone on the
//! same network can drive the simulated die.
//!
//! Messages are handled by the firmware loop between ticks, as the die's
//! main loop drains BLE writes.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use smokebomb_firmware::board;
use smokebomb_firmware::smokebomb_core::menu::PlayMode;
use smokebomb_hal::SecureElement;
use smokebomb_hal_simulator::SimSecureElement;
use smokebomb_shared::protocol::{DieToPhone, PhoneToDie};
use smokebomb_shared::types::MAX_POT_DICE;
use smokebomb_shared::{DieKind, LicensedItem, SignedRoll};
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;

/// How many rolls the simulator keeps for `SyncHistory`, as the die will
/// keep its recent rolls in flash.
pub const HISTORY_LEN: usize = 500;

/// The rolls since the simulator started, oldest first, at most
/// [`HISTORY_LEN`].
#[derive(Debug, Default)]
pub struct RollLog(VecDeque<SignedRoll>);

impl RollLog {
    pub fn push(&mut self, roll: SignedRoll) {
        if self.0.len() == HISTORY_LEN {
            self.0.pop_front();
        }
        self.0.push_back(roll);
    }

    /// The kept rolls from `since_counter` on, oldest first.
    pub fn since(&self, since_counter: u32) -> impl Iterator<Item = &SignedRoll> {
        self.0.iter().filter(move |r| r.record.counter >= since_counter)
    }
}

/// What the firmware loop gets from connected phones.
#[derive(Debug)]
pub enum PhoneRequest {
    /// A phone connected: greet it as the die does.
    Connected,
    Message(PhoneToDie),
}

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| session(socket, state))
}

async fn session(mut socket: WebSocket, state: AppState) {
    let mut rx = state.phone_out.subscribe();
    // The die's Bluetooth icon shows a connected phone.
    if state.phones.fetch_add(1, Ordering::SeqCst) == 0 {
        state.sim.lock().ble_connected = true;
    }
    tracing::info!("phone connected");
    let _ = state.phone_in.send(PhoneRequest::Connected);

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(json) => {
                    if socket.send(Message::Text(json.as_str().into())).await.is_err() { break; }
                }
                Err(RecvError::Lagged(n)) => tracing::debug!("phone lagged by {n} messages"),
                Err(RecvError::Closed) => break,
            },
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<PhoneToDie>(&text) {
                    Ok(m) => {
                        let _ = state.phone_in.send(PhoneRequest::Message(m));
                    }
                    Err(e) => tracing::warn!("bad phone message {text:?}: {e}"),
                },
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }

    tracing::info!("phone disconnected");
    if state.phones.fetch_sub(1, Ordering::SeqCst) == 1 {
        state.sim.lock().ble_connected = false;
    }
}

/// Send a message to every connected phone.
pub fn send(state: &AppState, msg: &DieToPhone) {
    let json = serde_json::to_string(msg).expect("DieToPhone serialises");
    // An error only means no phone is connected.
    let _ = state.phone_out.send(Arc::new(json));
}

/// Act on one request, as the die would on a BLE write.
pub fn handle(fw: &mut board::Firmware, history: &RollLog, state: &AppState, req: PhoneRequest) {
    let msg = match req {
        PhoneRequest::Connected => {
            send(state, &hello(state));
            send(state, &DieToPhone::Inventory(fw.inventory()));
            return;
        }
        PhoneRequest::Message(m) => m,
    };
    tracing::debug!("phone: {msg:?}");
    match msg {
        PhoneToDie::GetInventory => {}
        PhoneToDie::SetEnabledModes(modes) => fw.set_enabled_modes(modes),
        PhoneToDie::InstallLicense(license) => match license.item {
            // The simulator trusts the license; the die will check the
            // store's signature (docs/STORE.md).
            LicensedItem::Mode(mode) => fw.unlock_mode(mode),
            LicensedItem::Theme(_) => tracing::warn!("themes can't be installed on the simulator yet"),
        },
        PhoneToDie::SetDie { kind, count } => {
            let mut s = *fw.settings();
            if kind == DieKind::PassThePot {
                s.play = PlayMode::PassThePot;
                s.pot_count = count.clamp(1, MAX_POT_DICE as u8);
            } else {
                s.play = PlayMode::Dice;
                s.die = kind;
                s.count = count.clamp(1, kind.max_count() as u8);
            }
            fw.set_settings(s);
        }
        PhoneToDie::GetPublicKey => {
            if let Ok(key) = SimSecureElement::new().public_key() {
                send(state, &DieToPhone::PublicKey(key));
            }
        }
        PhoneToDie::SyncHistory { since_counter } => {
            for r in history.since(since_counter) {
                send(state, &DieToPhone::HistoryItem(Some(r.clone())));
            }
            send(state, &DieToPhone::HistoryItem(None));
            return;
        }
        PhoneToDie::SetOwnerName(_) | PhoneToDie::JoinSession(_) | PhoneToDie::LeaveSession => {
            tracing::info!("the simulator doesn't handle {msg:?} yet");
        }
    }
    // Every setup change is answered with the inventory, even when nothing
    // changed, so the app always hears back.
    send(state, &DieToPhone::Inventory(fw.inventory()));
}

fn hello(state: &AppState) -> DieToPhone {
    let v = |i: usize| {
        env!("CARGO_PKG_VERSION")
            .split('.')
            .nth(i)
            .and_then(|p| p.parse().ok())
            .unwrap_or(0)
    };
    DieToPhone::Hello {
        firmware_version: (v(0), v(1), v(2)),
        battery_percent: state.sim.lock().battery_percent,
    }
}

#[cfg(test)]
mod tests {
    use smokebomb_shared::{ModeId, ModeSet};
    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::protocol::StatusSnapshot;

    fn rig() -> (board::Firmware, AppState, broadcast::Receiver<Arc<String>>) {
        let sim = board::SimHandle::new();
        let fw = board::boot(&sim).unwrap();
        let (phone_in, _) = mpsc::unbounded_channel();
        let (phone_out, rx) = broadcast::channel(16);
        let state = AppState {
            sim,
            world: Arc::new(std::sync::Mutex::new(crate::world::World::new())),
            out: broadcast::channel(4).0,
            status: Arc::new(tokio::sync::Mutex::new(StatusSnapshot {
                mode: String::new(),
                die: "d20",
                die_count: 1,
                last_roll: None,
            })),
            phone_in,
            phone_out,
            phones: Arc::default(),
        };
        (fw, state, rx)
    }

    fn msg(json: &str) -> PhoneRequest {
        PhoneRequest::Message(serde_json::from_str(json).unwrap())
    }

    /// The JSON shapes the app's simulator link reads and writes.
    #[test]
    fn the_phone_turns_modes_off_and_hears_the_inventory() {
        let (mut fw, state, mut rx) = rig();
        handle(&mut fw, &RollLog::default(), &state, PhoneRequest::Connected);
        assert_eq!(
            *rx.try_recv().unwrap(),
            r#"{"Hello":{"firmware_version":[0,1,0],"battery_percent":78}}"#
        );
        assert_eq!(
            *rx.try_recv().unwrap(),
            r#"{"Inventory":{"licensed":15,"enabled":15,"active":"Dice"}}"#
        );

        handle(
            &mut fw,
            &RollLog::default(),
            &state,
            msg(r#"{"SetEnabledModes":4}"#),
        );
        assert_eq!(
            *rx.try_recv().unwrap(),
            r#"{"Inventory":{"licensed":15,"enabled":5,"active":"Dice"}}"#,
            "Dice stays on"
        );
        assert_eq!(fw.settings().modes(), ModeSet::DICE.with(ModeId::HotPotato));
    }

    #[test]
    fn the_phone_sets_the_dice() {
        let (mut fw, state, _rx) = rig();
        handle(
            &mut fw,
            &RollLog::default(),
            &state,
            msg(r#"{"SetDie":{"kind":"D6","count":3}}"#),
        );
        assert_eq!(fw.settings().active(), (DieKind::D6, 3));
        handle(
            &mut fw,
            &RollLog::default(),
            &state,
            msg(r#"{"SetDie":{"kind":"PassThePot","count":9}}"#),
        );
        assert_eq!(
            fw.settings().active(),
            (DieKind::PassThePot, 3),
            "at most three bills"
        );
    }

    fn roll(counter: u32) -> SignedRoll {
        use smokebomb_shared::{DeviceSerial, RollRecord, SessionId};
        SignedRoll {
            record: RollRecord {
                device: DeviceSerial([1; 9]),
                session: SessionId::NONE,
                counter,
                uptime_ms: 1000 * counter as u64,
                die: DieKind::D6,
                values: [counter as u8 % 6 + 1].into_iter().collect(),
                prev_hash: [0; 32],
            },
            signature: [counter as u8; 64],
        }
    }

    #[test]
    fn history_sync_sends_the_kept_rolls_from_the_counter_then_the_end() {
        let (mut fw, state, mut rx) = rig();
        let mut log = RollLog::default();
        for c in 0..3 {
            log.push(roll(c));
        }
        handle(
            &mut fw,
            &log,
            &state,
            msg(r#"{"SyncHistory":{"since_counter":1}}"#),
        );
        let got: Vec<DieToPhone> = std::iter::from_fn(|| rx.try_recv().ok())
            .map(|j| serde_json::from_str(&j).unwrap())
            .collect();
        assert_eq!(
            got,
            [
                DieToPhone::HistoryItem(Some(roll(1))),
                DieToPhone::HistoryItem(Some(roll(2))),
                DieToPhone::HistoryItem(None),
            ]
        );
        handle(
            &mut fw,
            &log,
            &state,
            msg(r#"{"SyncHistory":{"since_counter":9}}"#),
        );
        assert_eq!(*rx.try_recv().unwrap(), r#"{"HistoryItem":null}"#);
    }

    #[test]
    fn the_log_keeps_the_latest_rolls() {
        let mut log = RollLog::default();
        for c in 0..HISTORY_LEN as u32 + 5 {
            log.push(roll(c));
        }
        assert_eq!(log.since(0).count(), HISTORY_LEN);
        assert_eq!(log.since(0).next().unwrap().record.counter, 5);
    }
}
