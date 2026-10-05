//! Simulator <-> browser wire format. Mirrored in `web-ui/src/protocol.ts`.

use serde::{Deserialize, Serialize};
use smokebomb_hal::TargetId;
use smokebomb_shared::SignedRoll;

/// Bumped whenever a message changes shape. The browser compares it with its
/// own copy (`web-ui/src/protocol.ts`) and shows a banner when they differ,
/// which usually means the web UI build is out of date.
pub const PROTOCOL_VERSION: u32 = 5;

/// First byte of a binary frame packet. Then a header, the panel format
/// (1: 4 bpp grey, two pixels a byte, high nibble first; 2: RGB565, two
/// bytes a pixel, high byte first), its width and its height, one byte
/// each; then the six frames in `Face` order.
pub const FRAME_PACKET_TAG: u8 = 0x01;
/// Bytes before the first frame.
pub const FRAME_HEADER_LEN: usize = 4;
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

pub fn encode_frames(target: TargetId, faces: &[Vec<u8>; 6]) -> std::sync::Arc<Vec<u8>> {
    let side = match target {
        TargetId::Grey96 => 96,
        TargetId::Rgb64 => 64,
    };
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + faces.iter().map(Vec::len).sum::<usize>());
    buf.extend_from_slice(&[FRAME_PACKET_TAG, target as u8, side, side]);
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
        /// The die being simulated: 34 (96×96 grey) or 30 (64×64 colour) mm.
        die: u8,
    },
    /// The simulator swapped in the other die (and rebooted its firmware).
    Die {
        die: u8,
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
    /// Simulate the other die: 34 (96×96 grey) or 30 (64×64 colour) mm. The
    /// firmware reboots built for its panels.
    SetDie {
        die: u8,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_packets_say_their_panels() {
        let grey: [Vec<u8>; 6] = core::array::from_fn(|_| vec![0x5A; 96 * 96 / 2]);
        let p = encode_frames(TargetId::Grey96, &grey);
        assert_eq!(&p[..FRAME_HEADER_LEN], &[FRAME_PACKET_TAG, 1, 96, 96]);
        assert_eq!(p.len(), FRAME_HEADER_LEN + 6 * 4608);
        let rgb: [Vec<u8>; 6] = core::array::from_fn(|i| vec![i as u8; 64 * 64 * 2]);
        let p = encode_frames(TargetId::Rgb64, &rgb);
        assert_eq!(&p[..FRAME_HEADER_LEN], &[FRAME_PACKET_TAG, 2, 64, 64]);
        assert_eq!(p.len(), FRAME_HEADER_LEN + 6 * 8192);
        // Faces in order.
        assert_eq!(p[FRAME_HEADER_LEN + 5 * 8192], 5);
    }

    #[test]
    fn set_die_parses() {
        let m: Inbound = serde_json::from_str(r#"{"type":"set_die","die":30}"#).unwrap();
        assert!(matches!(m, Inbound::SetDie { die: 30 }));
    }
}
