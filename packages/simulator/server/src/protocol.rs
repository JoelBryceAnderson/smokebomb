//! Simulator <-> browser wire format. Mirrored in `web-ui/src/protocol.ts`.

use serde::{Deserialize, Serialize};
use smokebomb_hal::FRAME_BYTES;
use smokebomb_shared::SignedRoll;

/// Bumped whenever a message changes shape. The browser compares it with its
/// own copy (`web-ui/src/protocol.ts`) and shows a banner when they differ,
/// which usually means the web UI build is out of date.
pub const PROTOCOL_VERSION: u32 = 4;

/// First byte of a binary frame packet; followed by six 4bpp panel frames in
/// `Face` order.
pub const FRAME_PACKET_TAG: u8 = 0x01;
/// First byte of a binary pose packet; followed by seven little-endian f32:
/// rotation x, y, z, w (die body → world), then position x, y, z in scene
/// units.
pub const POSE_PACKET_TAG: u8 = 0x02;

#[derive(Clone, Debug)]
pub enum Outbound {
    Binary(std::sync::Arc<Vec<u8>>),
    Event(std::sync::Arc<String>),
}

pub fn encode_pose(pose: &crate::world::Pose) -> std::sync::Arc<Vec<u8>> {
    let r = pose.rotation;
    let p = pose.position;
    let mut buf = Vec::with_capacity(1 + 7 * 4);
    buf.push(POSE_PACKET_TAG);
    for v in [r.x, r.y, r.z, r.w, p.x, p.y, p.z] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    std::sync::Arc::new(buf)
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
    /// First message on every connection.
    Hello {
        protocol: u32,
        mode: String,
    },
    Mode {
        mode: String,
    },
    Haptic {
        effect: String,
    },
    /// The Nest's dock state changed (`OffNest`, `Seating`, `Ok`, `Wrong`,
    /// `NoPower`, `Display`).
    Nest {
        phase: String,
    },
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

/// Input from the browser. The die only moves through the world model.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Inbound {
    /// Finger on/off a face.
    Touch {
        face: u8,
        pressed: bool,
    },
    /// Start shaking in the hand (throw button pressed).
    ShakeStart,
    /// Stop shaking: `throw` releases the die, otherwise it's put down.
    ShakeEnd {
        throw: bool,
    },
    /// Quarter-turn tip. `right` is the viewer's right in world space.
    Tip {
        dir: TipDirection,
        right: [f32; 3],
    },
    /// Multi-turn: spin about a tip axis by `angle` radians since the spin
    /// began (positive yaw = right tip, negative pitch = up tip).
    Spin {
        axis: SpinAxis,
        angle: f32,
        right: [f32; 3],
    },
    /// Let go of a spin: the die settles on the nearest face.
    SpinEnd,
    /// Turn the die in the hand: radians about world Y, then world X.
    Rotate {
        yaw: f32,
        pitch: f32,
    },
    /// Set the die down with this face on top.
    PlaceFaceUp {
        face: u8,
    },
    Dock {
        docked: bool,
    },
    /// Put the die in the Nest with `face` down, turned `quarters` quarter
    /// turns about vertical.
    PlaceInNest {
        face: u8,
        quarters: u8,
    },
    /// Lift the die out of the Nest.
    Lift,
    /// A stray magnet beside the die.
    StrayMagnet {
        on: bool,
    },
    /// The Nest's cord is in a live socket.
    NestPlugged {
        on: bool,
    },
    /// Dirty contacts: no power gets through.
    DirtyContacts {
        on: bool,
    },
    ChargerFault {
        on: bool,
    },
    /// Set the battery level, 0-100.
    Battery {
        percent: u8,
    },
    /// Charge rate multiplier (1 = 1 %/min).
    ChargeRate {
        rate: f32,
    },
    /// Set the die's local time of day, seconds since midnight.
    SetTime {
        seconds: u32,
    },
    Ble {
        connected: bool,
    },
    ReducedMotion {
        on: bool,
    },
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TipDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum SpinAxis {
    Yaw,
    Pitch,
}

impl From<SpinAxis> for crate::world::SpinAxis {
    fn from(a: SpinAxis) -> Self {
        match a {
            SpinAxis::Yaw => Self::Yaw,
            SpinAxis::Pitch => Self::Pitch,
        }
    }
}

impl From<TipDirection> for crate::world::TipDir {
    fn from(d: TipDirection) -> Self {
        match d {
            TipDirection::Up => Self::Up,
            TipDirection::Down => Self::Down,
            TipDirection::Left => Self::Left,
            TipDirection::Right => Self::Right,
        }
    }
}
