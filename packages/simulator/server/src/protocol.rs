//! Simulator <-> browser wire format. Mirrored in `web-ui/src/protocol.ts`.

use serde::{Deserialize, Serialize};
use smokebomb_hal::FRAME_BYTES;
use smokebomb_shared::SignedRoll;

/// First byte of a binary frame packet; followed by six 4bpp panel frames in
/// `Face` order.
pub const FRAME_PACKET_TAG: u8 = 0x01;

#[derive(Clone, Debug)]
pub enum Outbound {
    Frames(std::sync::Arc<Vec<u8>>),
    Event(std::sync::Arc<String>),
}

impl Outbound {
    pub fn event(e: Event) -> Self {
        Outbound::Event(std::sync::Arc::new(
            serde_json::to_string(&e).expect("event serialises"),
        ))
    }
}

pub fn encode_frames(faces: &[[u8; FRAME_BYTES]; 6]) -> std::sync::Arc<Vec<u8>> {
    let mut buf = Vec::with_capacity(1 + FRAME_BYTES * 6);
    buf.push(FRAME_PACKET_TAG);
    for f in faces {
        buf.extend_from_slice(f);
    }
    std::sync::Arc::new(buf)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Mode { mode: String },
    Haptic { effect: String },
    Roll(RollView),
}

/// Field names match the backend's `POST /v1/rolls/verify` body, so a roll can
/// be piped straight to the server.
#[derive(Clone, Debug, Serialize)]
pub struct RollView {
    pub device_serial: String,
    pub counter: u32,
    pub uptime_ms: u64,
    pub die: &'static str,
    pub values: Vec<u8>,
    pub total: u16,
    pub digest: String,
    pub prev_hash: String,
    pub signature: String,
}

impl From<&SignedRoll> for RollView {
    fn from(r: &SignedRoll) -> Self {
        RollView {
            device_serial: hex::encode(r.record.device.0),
            counter: r.record.counter,
            uptime_ms: r.record.uptime_ms,
            die: r.record.die.wire_name(),
            values: r.record.values.to_vec(),
            total: r.record.total(),
            digest: hex::encode(r.record.digest()),
            prev_hash: hex::encode(r.record.prev_hash),
            signature: hex::encode(r.signature),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StatusSnapshot {
    pub mode: String,
    pub die: &'static str,
    pub die_count: u8,
    pub last_roll: Option<RollView>,
}

/// Input from the browser.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// Finger on/off a face.
    Touch {
        face: u8,
        pressed: bool,
    },
    /// Set which face points up at rest.
    Orient {
        up: u8,
    },
    /// Raw IMU override (becomes the resting sample).
    Imu {
        accel: [i16; 3],
        gyro: [i32; 3],
    },
    /// Canned motion; `land` picks the landing face for throws (random if absent).
    Gesture {
        kind: Gesture,
        land: Option<u8>,
    },
    Dock {
        docked: bool,
    },
    Ble {
        connected: bool,
    },
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Gesture {
    PickUp,
    Shake,
    Throw,
}
