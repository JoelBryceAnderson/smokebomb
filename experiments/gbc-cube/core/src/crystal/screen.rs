//! What Crystal is showing: the plain overworld, the overworld under a
//! text box or menu, a battle, or something else.
//!
//! Crystal composes every screen in `wTilemap` (20×18 tile IDs) before
//! copying it to VRAM. In the overworld that buffer is the map's metatiles
//! around the camera (`LoadOverworldTilemap`), with bit 7 of every ID
//! cleared. Text boxes and menus are drawn over it with font and frame
//! tiles, which are `$60` and up. So comparing `wTilemap` with the tiles the
//! map says should be there tells three things at once: whether the map
//! renderer's idea of the camera is right (everything matches), where any
//! text or menu is (the font tiles), and when the screen isn't the
//! overworld at all (lots of other mismatches: the bag, the Pokégear, the
//! title screen).

use crate::mem::GbMem;

use super::charmap;
use super::syms;
use super::world::{BlockCache, Camera, MapInfo};

pub const COLS: usize = 20;
pub const ROWS: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// Not a map screen (or not Crystal).
    Other,
    /// The map, nothing over it.
    Overworld,
    /// The map with text or a menu over part of it.
    OverworldUi,
    /// `wBattleMode` is set.
    Battle,
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenInfo {
    pub scene: Scene,
    /// Tiles of `wTilemap` that are text/menu: bit `x` of `ui[y]`.
    pub ui: [u32; ROWS],
    pub ui_tiles: u16,
    /// Tiles that are neither the expected map tile nor UI.
    pub stray_tiles: u16,
}

impl ScreenInfo {
    pub const fn other() -> ScreenInfo {
        ScreenInfo {
            scene: Scene::Other,
            ui: [0; ROWS],
            ui_tiles: 0,
            stray_tiles: 0,
        }
    }

    pub fn is_ui(&self, x: usize, y: usize) -> bool {
        self.ui[y] & (1 << x) != 0
    }
}

/// Up to this many tiles may disagree with the map and it's still the map
/// (a sprite-ish BG effect, a tile the game is redrawing).
const STRAY_LIMIT: u16 = 12;
/// At least this many tiles must be the map for it to be the map.
const MIN_MAP_TILES: u16 = 40;

/// A `wTilemap` tile.
pub fn tilemap<M: GbMem + ?Sized>(m: &M, x: usize, y: usize) -> u8 {
    m.byte(syms::W_TILEMAP.offset((y * COLS + x) as u16))
}

/// A `wAttrmap` attribute.
pub fn attrmap<M: GbMem + ?Sized>(m: &M, x: usize, y: usize) -> u8 {
    m.byte(syms::W_ATTRMAP.offset((y * COLS + x) as u16))
}

pub fn classify<M: GbMem + ?Sized>(
    m: &M,
    map: Option<&MapInfo>,
    cam: Option<&Camera>,
    cache: &mut BlockCache,
) -> ScreenInfo {
    let (Some(map), Some(cam)) = (map, cam) else {
        return ScreenInfo::other();
    };
    let mut info = ScreenInfo::other();
    for y in 0..ROWS {
        for x in 0..COLS {
            let (tx, ty) = (cam.anchor_tile.0 + x as i32, cam.anchor_tile.1 + y as i32);
            // `LoadMetatiles` draws the border block where the buffer is 0.
            let block = match map.block(m, tx.div_euclid(4), ty.div_euclid(4)) {
                Some(0) => map.border_block,
                Some(b) => b,
                None => {
                    info.stray_tiles += 1;
                    continue;
                }
            };
            let sub = (ty.rem_euclid(4) * 4 + tx.rem_euclid(4)) as usize;
            let want = cache.tile(m, map, block, sub).0;
            let got = tilemap(m, x, y);
            if got == want {
                continue;
            }
            if got >= charmap::UI_FIRST {
                info.ui[y] |= 1 << x;
                info.ui_tiles += 1;
            } else {
                info.stray_tiles += 1;
            }
        }
    }
    info.scene = if m.byte(syms::W_BATTLE_MODE) != 0 {
        Scene::Battle
    } else if info.stray_tiles > STRAY_LIMIT
        || ((COLS * ROWS) as u16 - info.ui_tiles - info.stray_tiles) < MIN_MAP_TILES
    {
        // Lots that isn't the map, or barely any map left showing (a screen
        // built only of font tiles, like the naming screen).
        Scene::Other
    } else if info.ui_tiles > 0 {
        Scene::OverworldUi
    } else {
        Scene::Overworld
    };
    info
}
