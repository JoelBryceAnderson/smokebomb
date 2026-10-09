//! Pokémon Crystal, read from its RAM.
//!
//! Everything here follows the pret/pokecrystal disassembly: addresses come
//! from its symbol file ([`syms`], generated) and the logic mirrors the
//! routines named in each function's docs, so a reader can check one against
//! the other. Constants that aren't symbols (struct layouts, charmap codes)
//! are copied here with the file they come from.

pub mod mons;
pub mod screen;
pub mod syms;
pub mod world;

/// `object_struct` fields (`constants/map_object_constants.asm`).
pub mod obj {
    pub const LENGTH: u16 = 0x28;
    pub const COUNT: u16 = 13;
    pub const SPRITE: u16 = 0x00;
    pub const SPRITE_TILE: u16 = 0x02;
    pub const FLAGS2: u16 = 0x05;
    pub const PALETTE: u16 = 0x06;
    pub const FACING: u16 = 0x0D;
    pub const MAP_X: u16 = 0x10;
    pub const MAP_Y: u16 = 0x11;
    pub const SPRITE_X: u16 = 0x17;
    pub const SPRITE_Y: u16 = 0x18;
    pub const SPRITE_X_OFFSET: u16 = 0x19;
    pub const SPRITE_Y_OFFSET: u16 = 0x1A;

    /// `OBJECT_FLAGS2` bits.
    pub const LOW_PRIORITY: u8 = 1 << 0;
    pub const HIGH_PRIORITY: u8 = 1 << 1;
    pub const IN_GRASS: u8 = 1 << 3;
    pub const USE_OBP1: u8 = 1 << 4;
    pub const UNDER_TILES: u8 = 1 << 7;

    /// `NUM_FACINGS`; `STANDING` (−1) and anything past it draw nothing.
    pub const NUM_FACINGS: u8 = 32;

    /// Facing template attribute bits (`RELATIVE_ATTRIBUTES_F`,
    /// `ABSOLUTE_TILE_ID_F`).
    pub const RELATIVE_ATTRIBUTES: u8 = 1 << 1;
    pub const ABSOLUTE_TILE_ID: u8 = 1 << 2;
}

/// Charmap codes for the screen's text (`constants/charmap.asm`).
pub mod charmap {
    /// The text box frame: ┌ ─ ┐ │ └ ┘.
    pub const FRAME_FIRST: u8 = 0x79;
    pub const FRAME_LAST: u8 = 0x7E;
    pub const SPACE: u8 = 0x7F;
    /// ▷ (inactive menu cursor) and ▶ (menu cursor).
    pub const CURSOR_HOLLOW: u8 = 0xEC;
    pub const CURSOR: u8 = 0xED;
    /// ▼, the "more text" prompt.
    pub const PROMPT: u8 = 0xEE;
    /// Tile IDs from here up are the font and the text box: never map tiles
    /// (map tiles are `$00`–`$5F` once bit 7 is cleared, see
    /// `_LoadOverworldAttrmapPals`).
    pub const UI_FIRST: u8 = 0x60;
}

/// The ROM bank holding the tileset palette maps: `_LoadOverworldAttrmapPals`
/// reads `wTilesetPalettes` with its own bank switched in.
pub const PALMAP_BANK: u8 = syms::LOAD_OVERWORLD_ATTRMAP_PALS.bank;

/// The game is waiting in `DelayFrame` for the next VBlank, so its RAM is a
/// finished picture of the next frame. (If the game's work overran the
/// frame, VBlank cleared the flag and the main loop hasn't set it again yet:
/// RAM may be half updated.)
pub fn settled<M: crate::mem::GbMem + ?Sized>(m: &M) -> bool {
    m.byte(syms::W_V_BLANK_OCCURRED) != 0
}

/// Is this ROM Pokémon Crystal? (Header title `PM_CRYSTAL`, any region or
/// revision; the RAM layout used here is the same in all of them, the ROM
/// symbols are checked in `tools/gen_syms.py` for 1.0 and 1.1.)
pub fn is_crystal(title: &str) -> bool {
    title.starts_with("PM_CRYSTAL")
}
