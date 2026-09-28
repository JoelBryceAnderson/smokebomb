//! Playback of the placeholder smoke clips from the QSPI asset pack.
//!
//! Superseded by the live particle system (SIM_SPEC decision H1); kept until
//! that lands so shaking and throwing still show something. Frames are
//! streamed from flash and added on top of each face.

use heapless::Vec;
use smokebomb_hal::{AssetStore, Face, FrameBytes, HalResult, FACE_COUNT};
use smokebomb_shared::assets::{ClipEntry, ClipId, SectionKind, CLIPS_HEADER_LEN, CLIP_ENTRY_LEN};

use crate::display::Framebuffer;
use crate::pack::PackIndex;

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
    /// Read the clip table, if the pack has one.
    pub fn load<A: AssetStore>(assets: &mut A, pack: &PackIndex) -> Self {
        let mut clips = Vec::new();
        if let Some(section) = pack.sections(SectionKind::Clips).next() {
            let mut hdr = [0u8; CLIPS_HEADER_LEN];
            if assets.read(section.offset, &mut hdr).is_ok() {
                let count = u16::from_le_bytes([hdr[0], hdr[1]]) as usize;
                for i in 0..count.min(MAX_CLIPS) {
                    let mut e = [0u8; CLIP_ENTRY_LEN];
                    let off = section.offset + (CLIPS_HEADER_LEN + i * CLIP_ENTRY_LEN) as u32;
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

    /// Add the current frame on top of `frames`. Returns `false` when
    /// nothing is playing.
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
            frames[face.index()].add_packed(scratch);
        }
        Ok(true)
    }
}
