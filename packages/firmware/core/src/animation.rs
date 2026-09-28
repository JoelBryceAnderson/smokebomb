//! Playback of pre-rendered smoke clips from the QSPI asset pack.
//!
//! Frames are streamed straight from flash into the framebuffers each tick;
//! nothing is decoded or composited on the MCU. See
//! [`smokebomb_shared::assets`] for the pack layout.

use heapless::Vec;
use smokebomb_hal::{AssetStore, Face, FrameBytes, HalResult, FACE_COUNT};
use smokebomb_shared::assets::{ClipEntry, ClipId, PackHeader, CLIP_ENTRY_LEN, HEADER_LEN, PACK_VERSION};

use crate::display::Framebuffer;

pub const MAX_CLIPS: usize = 32;

struct Playing {
    clip: ClipEntry,
    started_ms: Option<u64>,
}

pub struct AnimationPlayer {
    clips: Vec<ClipEntry, MAX_CLIPS>,
    playing: Option<Playing>,
}

impl AnimationPlayer {
    /// Read the clip table. A missing or corrupt pack yields a player with no
    /// clips; the die still rolls, just without smoke.
    pub fn load<A: AssetStore>(assets: &mut A) -> Self {
        let mut clips = Vec::new();
        let mut hdr = [0u8; HEADER_LEN];
        if assets.read(0, &mut hdr).is_ok() {
            if let Some(h) = PackHeader::decode(&hdr).filter(|h| h.version == PACK_VERSION) {
                for i in 0..(h.clip_count as usize).min(MAX_CLIPS) {
                    let mut e = [0u8; CLIP_ENTRY_LEN];
                    let off = (HEADER_LEN + i * CLIP_ENTRY_LEN) as u32;
                    if assets.read(off, &mut e).is_err() {
                        break;
                    }
                    let _ = clips.push(ClipEntry::decode(&e));
                }
            }
        }
        Self { clips, playing: None }
    }

    pub fn clip_count(&self) -> usize {
        self.clips.len()
    }

    pub fn play(&mut self, id: ClipId) {
        self.playing = self
            .clips
            .iter()
            .find(|c| c.id == id as u16)
            .map(|&clip| Playing {
                clip,
                started_ms: None,
            });
    }

    pub fn stop(&mut self) {
        self.playing = None;
    }

    /// Render the current frame into `frames`. Returns `false` when nothing is
    /// playing so the caller can draw static content instead.
    pub fn render<A: AssetStore>(
        &mut self,
        assets: &mut A,
        frames: &mut [Framebuffer; FACE_COUNT],
        scratch: &mut FrameBytes,
        now_ms: u64,
    ) -> HalResult<bool> {
        let Some(p) = self.playing.as_mut() else {
            return Ok(false);
        };
        let started = *p.started_ms.get_or_insert(now_ms);
        let fps = p.clip.fps.max(1) as u64;
        let mut frame = (now_ms - started) * fps / 1000;
        if frame >= p.clip.frame_count as u64 {
            if p.clip.looping() && p.clip.frame_count > 0 {
                frame %= p.clip.frame_count as u64;
            } else {
                self.playing = None;
                return Ok(false);
            }
        }
        let base = p.clip.frame_offset(frame as u16);
        for face in Face::ALL {
            assets.read(base + (face.index() * scratch.len()) as u32, scratch)?;
            frames[face.index()].load_packed(scratch);
        }
        Ok(true)
    }
}
