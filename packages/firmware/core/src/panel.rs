//! Sending frames to panels that can't take six whole frames a tick.
//!
//! The 64×64 RGB panels take 8 KB a frame over SPI at up to 10 MHz (SSD1357
//! datasheet Rev 1.0, Table 9-4): ≈6.6 ms a face, so six whole faces would
//! need ≈40 ms of bus a tick at 60 Hz. Most of a face rarely changes, so:
//!
//! 1. Each packed frame is cut into 8×8-pixel tiles and each tile hashed
//!    (FNV-1a over its bytes). A tile whose hash differs from the one last
//!    *sent* is dirty. Hashes cost 4 bytes a tile (256 B a face) instead of
//!    a 8 KB shadow copy; a collision (≈1 in 2³² per changed tile) would
//!    leave one tile stale until it next changes.
//! 2. A face's dirty tiles are sent as their bounding rectangle, through the
//!    panel's column/row address window ([`smokebomb_hal::Display::write_region`]).
//! 3. Faces are sent within a byte budget per tick, the one sent longest ago
//!    first and the face-down screen last and no more often than its own
//!    interval. A face that misses a tick stays dirty (its sent hashes are
//!    unchanged), so it goes next time.

use smokebomb_hal::{Face, Region, FACE_COUNT};

/// Tile side, pixels.
pub const TILE: usize = 8;

/// The tile hashes last sent to each face, and when each face was sent.
pub struct PanelSync<H: Copy + AsRef<[u32]> + AsMut<[u32]>> {
    sent: [H; FACE_COUNT],
    /// `false` until a face has been sent once, so the first frame goes
    /// whole whatever its hashes.
    primed: [bool; FACE_COUNT],
    sent_at: [u64; FACE_COUNT],
    /// Bytes sent so far, per face (for the budget report and tests).
    pub bytes: [u64; FACE_COUNT],
}

/// A packed frame's layout: `width`×`height` pixels, `bits` per pixel
/// (a multiple of 4), rows packed back to back.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    pub bits: usize,
}

impl Layout {
    fn tiles_x(&self) -> usize {
        self.width / TILE
    }

    fn tiles(&self) -> usize {
        self.tiles_x() * (self.height / TILE)
    }

    fn row_bytes(&self) -> usize {
        self.width * self.bits / 8
    }

    /// Bytes on the wire for `r`.
    pub fn region_bytes(&self, r: Region) -> usize {
        r.width() * self.bits / 8 * r.height()
    }
}

/// FNV-1a, 32-bit.
fn fnv(bytes: &[u8], mut h: u32) -> u32 {
    for &b in bytes {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Hash every tile of `frame` into `out`.
pub fn hash_tiles(frame: &[u8], layout: Layout, out: &mut [u32]) {
    let tile_bytes = TILE * layout.bits / 8;
    let row = layout.row_bytes();
    for (i, h) in out.iter_mut().enumerate().take(layout.tiles()) {
        let (tx, ty) = (i % layout.tiles_x(), i / layout.tiles_x());
        let mut acc = 0x811c_9dc5;
        for y in ty * TILE..(ty + 1) * TILE {
            let start = y * row + tx * tile_bytes;
            acc = fnv(&frame[start..start + tile_bytes], acc);
        }
        *h = acc;
    }
}

/// The bounding rectangle of the tiles where `now` differs from `sent`.
pub fn dirty_region(now: &[u32], sent: &[u32], layout: Layout) -> Region {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for i in 0..layout.tiles() {
        if now[i] != sent[i] {
            let (tx, ty) = (i % layout.tiles_x(), i / layout.tiles_x());
            x0 = x0.min(tx);
            y0 = y0.min(ty);
            x1 = x1.max(tx + 1);
            y1 = y1.max(ty + 1);
        }
    }
    if x0 == usize::MAX {
        return Region::default();
    }
    Region {
        x0: (x0 * TILE) as u8,
        y0: (y0 * TILE) as u8,
        x1: (x1 * TILE) as u8,
        y1: (y1 * TILE) as u8,
    }
}

/// What to send this tick: per face, its region (empty for none).
pub type Plan = [Region; FACE_COUNT];

impl<H: Copy + AsRef<[u32]> + AsMut<[u32]>> PanelSync<H> {
    pub const fn new(blank: H) -> Self {
        Self {
            sent: [blank; FACE_COUNT],
            primed: [false; FACE_COUNT],
            sent_at: [0; FACE_COUNT],
            bytes: [0; FACE_COUNT],
        }
    }

    /// Decide what to send, given each face's current tile hashes. `down`
    /// is the face on the table, sent last and at most every
    /// `down_every_ms`; `budget` is the bytes this tick may carry. A face
    /// whose region alone exceeds the budget still goes if it is the first
    /// sent this tick, so nothing starves.
    pub fn plan(
        &self,
        hashes: &[H; FACE_COUNT],
        layout: Layout,
        now_ms: u64,
        down: Face,
        down_every_ms: u64,
        budget: usize,
    ) -> Plan {
        let mut want = [Region::default(); FACE_COUNT];
        for f in 0..FACE_COUNT {
            want[f] = if self.primed[f] {
                dirty_region(hashes[f].as_ref(), self.sent[f].as_ref(), layout)
            } else {
                Region {
                    x0: 0,
                    y0: 0,
                    x1: layout.width as u8,
                    y1: layout.height as u8,
                }
            };
        }
        // Longest-unsent first; the face down after everything else.
        let mut order = [0usize, 1, 2, 3, 4, 5];
        order.sort_unstable_by_key(|&f| (f == down.index(), self.sent_at[f], f));
        let mut plan = [Region::default(); FACE_COUNT];
        let mut left = budget;
        let mut any = false;
        for f in order {
            let r = want[f];
            if r.is_empty() {
                continue;
            }
            if f == down.index() && self.primed[f] && now_ms.saturating_sub(self.sent_at[f]) < down_every_ms {
                continue;
            }
            let cost = layout.region_bytes(r);
            if cost <= left || !any {
                plan[f] = r;
                left = left.saturating_sub(cost);
                any = true;
            }
        }
        plan
    }

    /// Record that `region` of `face` was sent with these tile hashes.
    pub fn sent(&mut self, face: usize, region: Region, hashes: &H, layout: Layout, now_ms: u64) {
        if region.is_empty() {
            return;
        }
        let tx = layout.tiles_x();
        for ty in region.y0 as usize / TILE..region.y1 as usize / TILE {
            for x in region.x0 as usize / TILE..region.x1 as usize / TILE {
                self.sent[face].as_mut()[ty * tx + x] = hashes.as_ref()[ty * tx + x];
            }
        }
        self.primed[face] = true;
        self.sent_at[face] = now_ms;
        self.bytes[face] += layout.region_bytes(region) as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Layout = Layout {
        width: 64,
        height: 64,
        bits: 16,
    };

    fn hashes(frame: &[u8]) -> [u32; 64] {
        let mut h = [0u32; 64];
        hash_tiles(frame, L, &mut h);
        h
    }

    fn set(frame: &mut [u8], x: usize, y: usize) {
        frame[(y * 64 + x) * 2] ^= 0xFF;
    }

    #[test]
    fn a_changed_pixel_dirties_its_tile_only() {
        let a = [0u8; 8192];
        let mut b = a;
        set(&mut b, 17, 40);
        let r = dirty_region(&hashes(&b), &hashes(&a), L);
        assert_eq!(
            r,
            Region {
                x0: 16,
                y0: 40,
                x1: 24,
                y1: 48
            }
        );
        assert_eq!(L.region_bytes(r), 128);
        assert!(dirty_region(&hashes(&a), &hashes(&a), L).is_empty());
    }

    #[test]
    fn the_region_bounds_every_dirty_tile() {
        let a = [0u8; 8192];
        let mut b = a;
        set(&mut b, 0, 0);
        set(&mut b, 63, 63);
        assert_eq!(
            dirty_region(&hashes(&b), &hashes(&a), L),
            Region::full::<smokebomb_hal::Rgb64>()
        );
    }

    #[test]
    fn first_frame_goes_whole_then_only_changes() {
        let mut sync = PanelSync::new([0u32; 64]);
        let frame = [0u8; 8192];
        let h = [hashes(&frame); 6];
        let plan = sync.plan(&h, L, 0, Face::NegY, 250, usize::MAX);
        assert!(plan.iter().all(|r| *r == Region::full::<smokebomb_hal::Rgb64>()));
        for f in 0..6 {
            sync.sent(f, plan[f], &h[f], L, 0);
        }
        let plan = sync.plan(&h, L, 16, Face::NegY, 250, usize::MAX);
        assert!(plan.iter().all(|r| r.is_empty()), "nothing changed");
    }

    #[test]
    fn the_budget_defers_faces_and_they_catch_up() {
        let mut sync = PanelSync::new([0u32; 64]);
        let blank = [hashes(&[0u8; 8192]); 6];
        let plan = sync.plan(&blank, L, 0, Face::NegY, 0, usize::MAX);
        for f in 0..6 {
            sync.sent(f, plan[f], &blank[f], L, 0);
        }
        // Every face changes everywhere; the budget fits two.
        let busy = [hashes(&[1u8; 8192]); 6];
        let plan = sync.plan(&busy, L, 16, Face::NegY, 0, 2 * 8192);
        let sent: std::vec::Vec<usize> = (0..6).filter(|&f| !plan[f].is_empty()).collect();
        assert_eq!(sent.len(), 2);
        assert!(!sent.contains(&Face::NegY.index()), "the face down goes last");
        for &f in &sent {
            sync.sent(f, plan[f], &busy[f], L, 16);
        }
        let next = sync.plan(&busy, L, 33, Face::NegY, 0, 2 * 8192);
        for f in sent {
            assert!(next[f].is_empty(), "already sent");
        }
        assert_eq!(next.iter().filter(|r| !r.is_empty()).count(), 2);
    }

    #[test]
    fn the_face_down_waits_for_its_interval() {
        let mut sync = PanelSync::new([0u32; 64]);
        let a = [hashes(&[0u8; 8192]); 6];
        let plan = sync.plan(&a, L, 0, Face::NegY, 250, usize::MAX);
        for f in 0..6 {
            sync.sent(f, plan[f], &a[f], L, 0);
        }
        let b = [hashes(&[2u8; 8192]); 6];
        let down = Face::NegY.index();
        assert!(sync.plan(&b, L, 100, Face::NegY, 250, usize::MAX)[down].is_empty());
        assert!(!sync.plan(&b, L, 300, Face::NegY, 250, usize::MAX)[down].is_empty());
    }

    #[test]
    fn an_oversized_region_still_goes_alone() {
        let sync = PanelSync::new([0u32; 64]);
        let h = [hashes(&[0u8; 8192]); 6];
        let plan = sync.plan(&h, L, 0, Face::NegY, 0, 100);
        assert_eq!(plan.iter().filter(|r| !r.is_empty()).count(), 1);
    }

    #[test]
    fn grey_4bpp_tiles_hash_whole_bytes() {
        let l = Layout {
            width: 96,
            height: 96,
            bits: 4,
        };
        let a = [0u8; 4608];
        let mut b = a;
        b[48 * 48 + 20] = 0x0F; // row 48, pixel 41
        let (mut ha, mut hb) = ([0u32; 144], [0u32; 144]);
        hash_tiles(&a, l, &mut ha);
        hash_tiles(&b, l, &mut hb);
        let r = dirty_region(&hb, &ha, l);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (40, 48, 48, 56));
        assert_eq!(l.region_bytes(r), 32);
    }
}
