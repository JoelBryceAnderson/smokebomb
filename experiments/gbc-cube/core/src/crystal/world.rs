//! Phase 3: the overworld redrawn from game state, past the screen's edges.
//!
//! Crystal keeps the whole current map in WRAM as blocks
//! (`wOverworldMapBlocks`, with a three-block border that holds the strips
//! of connected maps), and draws the screen from it: each block is a 4×4
//! "metatile" of tile IDs read from ROM, each tile ID's palette comes from
//! the tileset's palette map, and the tile graphics and colours are in VRAM
//! and palette RAM. Doing the same for a 192×192 window gives the side faces
//! real map instead of the 160×144 frame's edges.
//!
//! Timing. When `run_frame` returns the PPU has just finished frame N and
//! the game is halted in `DelayFrame`, having already set up everything for
//! frame N+1: WRAM, `hSCX`/`hSCY` and the shadow OAM. Its VBlank handler
//! copies those to the hardware at the start of the next call. This module
//! reads the prepared state, so its picture is frame N+1's, one frame
//! ahead of the emulator's (and the debug overlay compares it with N+1).

use crate::color::palette_from_ram;
use crate::mem::{reg, GbMem, Sym};
use crate::ppu::{self, attr, bg_tile_offset, tile_row_packed, Obj, BG_PRIORITY};
use crate::view::{View, MID, V, VOID};

use super::{obj, syms, PALMAP_BANK};

/// `wOverworldMapBlocks` is `ds 1300`.
const BLOCK_BUFFER: usize = 1300;
/// `MAP_CONNECTION_PADDING`: blocks of border around the map on each side.
pub const PADDING: usize = 3;

/// The current map as `wOverworldMapBlocks` holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapInfo {
    /// Blocks per row of the buffer: map width + 6.
    pub stride: usize,
    /// Rows in the buffer: map height + 6.
    pub rows: usize,
    pub border_block: u8,
    pub blocks_bank: u8,
    pub blocks_addr: u16,
    pub palmap_addr: u16,
}

impl MapInfo {
    pub fn read<M: GbMem + ?Sized>(m: &M) -> Option<MapInfo> {
        let w = m.byte(syms::W_MAP_WIDTH) as usize;
        let h = m.byte(syms::W_MAP_HEIGHT) as usize;
        let stride = w + 2 * PADDING;
        let rows = h + 2 * PADDING;
        let blocks_addr = m.word(syms::W_TILESET_BLOCKS_ADDRESS);
        let palmap_addr = m.word(syms::W_TILESET_PALETTES);
        if w == 0 || h == 0 || stride * rows > BLOCK_BUFFER {
            return None;
        }
        if !(0x4000..0x8000).contains(&blocks_addr) || !(0x4000..0x8000).contains(&palmap_addr) {
            return None;
        }
        Some(MapInfo {
            stride,
            rows,
            border_block: m.byte(syms::W_MAP_BORDER_BLOCK),
            blocks_bank: m.byte(syms::W_TILESET_BLOCKS_BANK),
            blocks_addr,
            palmap_addr,
        })
    }

    /// The block at block coordinates (`bx`, `by`) of the buffer, `None`
    /// outside it. 0 means "no map here": the buffer starts zeroed
    /// (`LoadBlockData`) and only the map and its connection strips are
    /// copied in, so 0 is the border the game fills in with
    /// `wMapBorderBlock` when it draws.
    pub fn block<M: GbMem + ?Sized>(&self, m: &M, bx: i32, by: i32) -> Option<u8> {
        if bx < 0 || by < 0 || bx as usize >= self.stride || by as usize >= self.rows {
            return None;
        }
        let off = by as usize * self.stride + bx as usize;
        Some(m.byte(syms::W_OVERWORLD_MAP_BLOCKS.offset(off as u16)))
    }
}

/// Metatiles and their palette nibbles, read from ROM once per tileset.
pub struct BlockCache {
    key: (u8, u16, u16),
    valid: [bool; 128],
    /// Per block, 16 × (tile ID, attributes).
    tiles: [[(u8, u8); 16]; 128],
}

impl Default for BlockCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockCache {
    pub const fn new() -> Self {
        BlockCache {
            key: (0, 0, 0),
            valid: [false; 128],
            tiles: [[(0, 0); 16]; 128],
        }
    }

    /// Tile ID (bit 7 cleared, as the game stores it in `wTilemap`) and
    /// attributes of sub-tile `sub` (row-major in the 4×4) of `block`.
    pub fn tile<M: GbMem + ?Sized>(&mut self, m: &M, map: &MapInfo, block: u8, sub: usize) -> (u8, u8) {
        let key = (map.blocks_bank, map.blocks_addr, map.palmap_addr);
        if key != self.key {
            self.key = key;
            self.valid = [false; 128];
        }
        // `LoadMetatiles` doubles the block number in 8 bits before scaling
        // it, so blocks 128+ wrap onto 0+ (a known bug, kept).
        let b = (block & 0x7F) as usize;
        if !self.valid[b] {
            for (i, t) in self.tiles[b].iter_mut().enumerate() {
                let raw = m.rom_banked(map.blocks_bank, map.blocks_addr.wrapping_add((b * 16 + i) as u16));
                let nib = m.rom_banked(PALMAP_BANK, map.palmap_addr.wrapping_add(raw as u16 / 2));
                let a = if raw & 1 == 0 { nib & 0x0F } else { nib >> 4 };
                *t = (raw & 0x7F, a);
            }
            self.valid[b] = true;
        }
        self.tiles[b][sub]
    }
}

/// Where the screen is on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Camera {
    /// Map pixel (from the buffer's top-left) of the screen's top-left,
    /// including the smooth scroll of a step in progress.
    pub screen: (i32, i32),
    /// Map tile of `wTilemap`'s top-left (no scroll).
    pub anchor_tile: (i32, i32),
    /// Screen pixel at the centre of the player's 16×16 ground cell. The
    /// player's sprite is drawn 4 px higher (it stands on the cell).
    pub centre: (i32, i32),
    /// The player's ground cell's top-left on screen, as the game's 8-bit
    /// sprite coordinates.
    player_cell: (u8, u8),
}

impl Camera {
    /// From `wOverworldMapAnchor`, `wPlayerMetatileX/Y`, `wBGMapAnchor` and
    /// `hSCX/hSCY`.
    ///
    /// At the start of a step `UpdateOverworldMap` moves both anchors a
    /// whole 16 px step ahead and redraws the BG map there; then
    /// `ScrollScreen` walks `hSCX`/`hSCY` toward it a pixel or two a frame.
    /// So the screen's map position is the anchor's, plus how far the
    /// scroll still is from the anchor's tile in the BG map.
    pub fn read<M: GbMem + ?Sized>(m: &M, map: &MapInfo) -> Option<Camera> {
        let anchor = m.word(syms::W_OVERWORLD_MAP_ANCHOR);
        let off = anchor.wrapping_sub(syms::W_OVERWORLD_MAP_BLOCKS.addr) as usize;
        if off >= map.stride * map.rows {
            return None;
        }
        let (mx, my) = (
            m.byte(syms::W_PLAYER_METATILE_X),
            m.byte(syms::W_PLAYER_METATILE_Y),
        );
        if mx > 1 || my > 1 {
            return None;
        }
        let tx = (off % map.stride) as i32 * 4 + mx as i32 * 2;
        let ty = (off / map.stride) as i32 * 4 + my as i32 * 2;
        let bg = m.word(syms::W_BG_MAP_ANCHOR);
        if !(0x9800..0x9C00).contains(&bg) {
            return None;
        }
        let bgx = ((bg & 0x1F) * 8) as u8;
        let bgy = (((bg >> 5) & 0x1F) * 8) as u8;
        let dx = m.byte(syms::H_SCX).wrapping_sub(bgx) as i8 as i32;
        let dy = m.byte(syms::H_SCY).wrapping_sub(bgy) as i8 as i32;
        if dx.abs() > 16 || dy.abs() > 16 {
            return None;
        }
        // The player's sprite position (`InitSprite`, without the per-frame
        // offsets that make it hop down ledges), its own top-left.
        let p = syms::W_OBJECT_STRUCTS;
        let cell = (
            m.byte(p.offset(obj::SPRITE_X))
                .wrapping_add(m.byte(syms::W_PLAYER_BG_MAP_OFFSET_X)),
            m.byte(p.offset(obj::SPRITE_Y))
                .wrapping_add(m.byte(syms::W_PLAYER_BG_MAP_OFFSET_Y)),
        );
        let mut centre = (cell.0 as i32 + 8, cell.1 as i32 + 8);
        if !(0..160).contains(&centre.0) || !(0..144).contains(&centre.1) {
            // A player sprite parked off screen (some cutscenes): keep the
            // usual spot. 64, 64 is where `InitXCoord`/`InitYCoord` put the
            // player: (object X − wXCoord) × 16 with the player object 4
            // steps in.
            centre = (72, 72);
        }
        Some(Camera {
            screen: (tx * 8 + dx, ty * 8 + dy),
            anchor_tile: (tx, ty),
            centre,
            player_cell: cell,
        })
    }

    /// Map pixel at the centre of the up face.
    pub fn centre_on_map(&self) -> (i32, i32) {
        (self.screen.0 + self.centre.0, self.screen.1 + self.centre.1)
    }

    /// Canvas position of the screen's top-left pixel.
    pub fn screen_on_canvas(&self) -> (i32, i32) {
        (MID - self.centre.0, MID - self.centre.1)
    }
}

/// Draw the map around the player into `view`, objects included.
pub fn draw<M: GbMem + ?Sized>(m: &M, map: &MapInfo, cam: &Camera, cache: &mut BlockCache, view: &mut View) {
    view.palette = palette_from_ram(m.bg_palette(), m.obj_palette());
    let lcdc = m.reg(reg::LCDC);
    let (cx, cy) = cam.centre_on_map();
    let (ox, oy) = (cx - MID, cy - MID);
    let vram = m.vram();
    let (tx0, tx1) = (ox.div_euclid(8), (ox + V as i32 - 1).div_euclid(8));
    let (ty0, ty1) = (oy.div_euclid(8), (oy + V as i32 - 1).div_euclid(8));
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let block = map.block(m, tx.div_euclid(4), ty.div_euclid(4)).unwrap_or(0);
            let tile = if block == 0 {
                None
            } else {
                let sub = (ty.rem_euclid(4) * 4 + tx.rem_euclid(4)) as usize;
                Some(cache.tile(m, map, block, sub))
            };
            let x0 = tx * 8 - ox;
            for r in 0..8 {
                let y = ty * 8 + r - oy;
                if !(0..V as i32).contains(&y) {
                    continue;
                }
                let row = &mut view.idx[y as usize * V..(y as usize + 1) * V];
                // Eight pixels at once: colour numbers plus the palette
                // (and priority) bits in every byte.
                let px = tile.map_or(u64::from_le_bytes([VOID; 8]), |(t, a)| {
                    let base =
                        ((a & attr::PALETTE) << 2) | if a & attr::PRIORITY != 0 { BG_PRIORITY } else { 0 };
                    tile_row_packed(vram, bg_tile_offset(lcdc, t, a), r as usize, a, 8)
                        | u64::from_le_bytes([base; 8])
                });
                if x0 >= 0 && x0 + 8 <= V as i32 {
                    row[x0 as usize..x0 as usize + 8].copy_from_slice(&px.to_le_bytes());
                } else {
                    for (i, b) in px.to_le_bytes().into_iter().enumerate() {
                        let x = x0 + i as i32;
                        if (0..V as i32).contains(&x) {
                            row[x as usize] = b;
                        }
                    }
                }
            }
        }
    }
    let mut objs = [Obj::default(); 40];
    let n = objects(m, cam, &mut objs);
    ppu::draw_objs(m, view, &objs[..n], cam.screen_on_canvas());
}

/// The game's objects as OAM entries, the way `InitSprites` /
/// `.InitSprite` (`engine/overworld/map_objects.asm`) builds the shadow OAM
/// for the next frame, but for every object struct with a sprite, on screen
/// or not: off-screen objects get positions off the screen instead of
/// wrapping around it. Returns how many entries were written.
pub fn objects<M: GbMem + ?Sized>(m: &M, cam: &Camera, out: &mut [Obj; 40]) -> usize {
    const HIGH: u8 = 3;
    const NORM: u8 = 2;
    const LOW: u8 = 1;
    let field = |i: u16, f: u16| m.byte(syms::W_OBJECT_STRUCTS.offset(i * obj::LENGTH + f));
    let off = (
        m.byte(syms::W_PLAYER_BG_MAP_OFFSET_X),
        m.byte(syms::W_PLAYER_BG_MAP_OFFSET_Y),
    );
    // `.DeterminePriorities`
    let mut prio = [0u8; obj::COUNT as usize];
    for i in 0..obj::COUNT {
        if field(i, obj::SPRITE) == 0 || field(i, obj::FACING) >= obj::NUM_FACINGS {
            continue;
        }
        let f2 = field(i, obj::FLAGS2);
        prio[i as usize] = if f2 & obj::LOW_PRIORITY != 0 {
            LOW
        } else if f2 & obj::HIGH_PRIORITY != 0 {
            HIGH
        } else {
            NORM
        };
    }
    let mut n = 0;
    for level in [HIGH, NORM, LOW] {
        for i in 0..obj::COUNT {
            if prio[i as usize] != level {
                continue;
            }
            // `.InitSprite`
            let st = field(i, obj::SPRITE_TILE);
            let tile = st & 0x7F;
            let f2 = field(i, obj::FLAGS2);
            let mut d = if st & 0x80 == 0 { attr::BANK1 } else { 0 };
            if f2 & obj::UNDER_TILES != 0 {
                d |= attr::PRIORITY;
            }
            if f2 & obj::USE_OBP1 != 0 {
                d |= attr::OBP1;
            }
            d |= field(i, obj::PALETTE) & attr::PALETTE;
            let flags = if f2 & obj::IN_GRASS != 0 {
                attr::PRIORITY
            } else {
                0
            };
            let x = field(i, obj::SPRITE_X)
                .wrapping_add(field(i, obj::SPRITE_X_OFFSET))
                .wrapping_add(off.0);
            let y = field(i, obj::SPRITE_Y)
                .wrapping_add(field(i, obj::SPRITE_Y_OFFSET))
                .wrapping_sub(4)
                .wrapping_add(off.1);
            // Where the map says it is, relative to the player, in pixels.
            let map_dx = field(i, obj::MAP_X).wrapping_sub(field(0, obj::MAP_X)) as i8 as i32 * 16;
            let map_dy = field(i, obj::MAP_Y).wrapping_sub(field(0, obj::MAP_Y)) as i8 as i32 * 16;
            let facing = field(i, obj::FACING);
            let table = Sym::new(syms::FACINGS.bank, syms::FACINGS.addr + facing as u16 * 2);
            let entry = m.word(table);
            let count = m.rom_banked(syms::FACINGS.bank, entry);
            for k in 0..count as u16 {
                if n == out.len() {
                    return n; // `.full`
                }
                let e = |j: u16| m.rom_banked(syms::FACINGS.bank, entry + 1 + 4 * k + j);
                let (ey, ex, ea, et) = (e(0), e(1), e(2), e(3));
                let t = if ea & obj::ABSOLUTE_TILE_ID != 0 { 0 } else { tile }.wrapping_add(et);
                let a = if ea & obj::RELATIVE_ATTRIBUTES != 0 {
                    flags | ea
                } else {
                    ea
                };
                let a = (a & (attr::OBP1 | attr::XFLIP | attr::YFLIP | attr::PRIORITY)) | d;
                // Screen positions are 8-bit: read them relative to the
                // player, taking the wrap the map coordinates agree with, so
                // an object off screen stays off screen.
                let rx = unwrap(x.wrapping_add(ex).wrapping_sub(cam.player_cell.0), map_dx);
                let ry = unwrap(y.wrapping_add(ey).wrapping_sub(cam.player_cell.1), map_dy);
                out[n] = Obj {
                    x: cam.player_cell.0 as i32 + rx,
                    y: cam.player_cell.1 as i32 + ry,
                    tile: t,
                    attr: a,
                };
                n += 1;
            }
        }
    }
    n
}

/// The 8-bit offset `d` as the signed offset nearest `expect`.
fn unwrap(d: u8, expect: i32) -> i32 {
    let d = d as i8 as i32;
    [d - 256, d, d + 256]
        .into_iter()
        .min_by_key(|v| (v - expect).abs())
        .unwrap_or(d)
}
