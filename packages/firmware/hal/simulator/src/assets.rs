//! Placeholder asset pack so the simulator shows *something* before real
//! pre-rendered smoke exists. Frames are cheap procedural rings; the real
//! pack comes from the offline render pipeline.

use smokebomb_shared::assets::*;
use smokebomb_shared::types::FACE_COUNT;

struct Clip {
    id: ClipId,
    frames: u16,
    looping: bool,
}

const CLIPS: [Clip; 5] = [
    Clip {
        id: ClipId::SmokeIdle,
        frames: 30,
        looping: true,
    },
    Clip {
        id: ClipId::SmokeShake,
        frames: 20,
        looping: true,
    },
    Clip {
        id: ClipId::SmokeThrow,
        frames: 15,
        looping: true,
    },
    Clip {
        id: ClipId::MaxBurst,
        frames: 30,
        looping: false,
    },
    Clip {
        id: ClipId::Dud,
        frames: 15,
        looping: false,
    },
];

pub fn placeholder_pack() -> Vec<u8> {
    let table_len = HEADER_LEN + CLIPS.len() * CLIP_ENTRY_LEN;
    let mut entries = Vec::new();
    let mut data = Vec::new();
    for clip in &CLIPS {
        entries.push(ClipEntry {
            id: clip.id as u16,
            frame_count: clip.frames,
            fps: 30,
            flags: if clip.looping { ClipEntry::FLAG_LOOP } else { 0 },
            offset: (table_len + data.len()) as u32,
        });
        for f in 0..clip.frames {
            for face in 0..FACE_COUNT {
                data.extend(ring_frame(clip.id, f, clip.frames, face));
            }
        }
    }

    let total_len = (table_len + data.len()) as u32;
    let mut out = Vec::with_capacity(total_len as usize);
    out.extend(
        PackHeader {
            version: PACK_VERSION,
            clip_count: CLIPS.len() as u16,
            total_len,
        }
        .encode(),
    );
    for e in &entries {
        out.extend(e.encode());
    }
    out.extend(data);
    out
}

/// Expanding soft ring, phase-shifted per face so the cube shimmers.
fn ring_frame(clip: ClipId, frame: u16, frames: u16, face: usize) -> Vec<u8> {
    let t = (frame as f32 / frames as f32 + face as f32 / 6.0) % 1.0;
    let radius = 8.0 + t * 48.0;
    let width = match clip {
        ClipId::MaxBurst => 10.0,
        ClipId::SmokeShake => 4.0,
        _ => 7.0,
    };
    let peak = if clip == ClipId::Dud {
        5.0
    } else {
        15.0 * (1.0 - t * 0.6)
    };
    let mut buf = vec![0u8; FRAME_BYTES];
    for y in 0..PANEL_HEIGHT {
        for x in 0..PANEL_WIDTH {
            let dx = x as f32 - 47.5;
            let dy = y as f32 - 47.5;
            let d = (dx * dx + dy * dy).sqrt();
            let level = (peak * (1.0 - ((d - radius).abs() / width)).max(0.0)) as u8;
            let i = y * PANEL_WIDTH + x;
            buf[i / 2] |= if i % 2 == 0 { level << 4 } else { level & 0x0f };
        }
    }
    buf
}
