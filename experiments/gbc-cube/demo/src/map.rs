//! The demo map, "Cube Town": metatiles and the block layout, in the same
//! form Crystal keeps its maps (4×4-tile blocks, a 3-block border holding
//! the connected map's strip).

use crate::art::bg;

/// Map size in blocks.
pub const W: usize = 20;
pub const H: usize = 18;
pub const PAD: usize = 3;
pub const STRIDE: usize = W + 2 * PAD;
pub const ROWS: usize = H + 2 * PAD;

pub mod block {
    pub const GRASS: u8 = 1;
    pub const PATH: u8 = 2;
    pub const TALL: u8 = 3;
    pub const FLOWERS: u8 = 4;
    pub const WATER: u8 = 5;
    pub const TREES: u8 = 6;
    pub const HOUSE_TL: u8 = 7;
    pub const HOUSE_TR: u8 = 8;
    pub const HOUSE_BL: u8 = 9;
    pub const HOUSE_BR: u8 = 10;
    pub const FENCE: u8 = 11;
    pub const SIGN: u8 = 12;
    pub const COUNT: usize = 13;
}

/// The 16 tile IDs of each block, row-major.
pub fn metatiles() -> [[u8; 16]; block::COUNT] {
    use bg::*;
    let mut m = [[0u8; 16]; block::COUNT];
    m[block::GRASS as usize] = [
        GRASS, GRASS_B, GRASS, GRASS, GRASS, GRASS, GRASS_B, GRASS, GRASS_B, GRASS, GRASS, GRASS, GRASS,
        GRASS, GRASS, GRASS_B,
    ];
    m[block::PATH as usize] = [
        PATH, PATH, PATH_B, PATH, PATH, PATH_B, PATH, PATH, PATH, PATH, PATH, PATH_B, PATH_B, PATH, PATH,
        PATH,
    ];
    m[block::TALL as usize] = [TALL_GRASS; 16];
    m[block::FLOWERS as usize] = [
        FLOWER, GRASS, FLOWER, GRASS, GRASS, FLOWER, GRASS_B, FLOWER, FLOWER, GRASS, FLOWER, GRASS, GRASS_B,
        FLOWER, GRASS, FLOWER,
    ];
    m[block::WATER as usize] = [WATER; 16];
    m[block::TREES as usize] = [
        TREE_TL, TREE_TR, TREE_TL, TREE_TR, TREE_BL, TREE_BR, TREE_BL, TREE_BR, TREE_TL, TREE_TR, TREE_TL,
        TREE_TR, TREE_BL, TREE_BR, TREE_BL, TREE_BR,
    ];
    // An 8×8-tile house over four blocks.
    let house: [[u8; 8]; 8] = [
        [ROOF_L, ROOF, ROOF, ROOF, ROOF, ROOF, ROOF, ROOF_R],
        [ROOF; 8],
        [ROOF; 8],
        [ROOF; 8],
        [WALL; 8],
        [WALL, WINDOW, WALL, WALL, WALL, WALL, WINDOW, WALL],
        [WALL, WALL, WALL, DOOR, WALL, WALL, WALL, WALL],
        [WALL, WINDOW, WALL, DOOR, WALL, WALL, WINDOW, WALL],
    ];
    for (b, (bx, by)) in [
        (block::HOUSE_TL, (0, 0)),
        (block::HOUSE_TR, (4, 0)),
        (block::HOUSE_BL, (0, 4)),
        (block::HOUSE_BR, (4, 4)),
    ] {
        for y in 0..4 {
            for x in 0..4 {
                m[b as usize][y * 4 + x] = house[by + y][bx + x];
            }
        }
    }
    m[block::FENCE as usize] = [
        FENCE, FENCE, FENCE, FENCE, FENCE, FENCE, FENCE, FENCE, GRASS, GRASS_B, GRASS, GRASS, GRASS, GRASS,
        GRASS, GRASS_B,
    ];
    m[block::SIGN as usize] = [
        GRASS, GRASS, GRASS, GRASS_B, GRASS, GRASS, GRASS, GRASS, GRASS_B, SIGN, SIGN, GRASS, GRASS, SIGN,
        SIGN, GRASS,
    ];
    m
}

/// `wOverworldMapBlocks` for Cube Town: the map in the middle, a strip of
/// the road north of town in the top border, zero (no map) elsewhere.
pub fn blocks() -> [u8; STRIDE * ROWS] {
    use block::*;
    let mut town = [[GRASS; W]; H];
    for (y, row) in town.iter_mut().enumerate() {
        for (x, b) in row.iter_mut().enumerate() {
            let edge = x == 0 || y == 0 || x == W - 1 || y == H - 1;
            if edge && !(y == 0 && (9..=10).contains(&x)) {
                *b = TREES;
            }
        }
    }
    let fill = |town: &mut [[u8; W]; H], x0: usize, y0: usize, x1: usize, y1: usize, b: u8| {
        for row in town.iter_mut().take(y1 + 1).skip(y0) {
            for v in row.iter_mut().take(x1 + 1).skip(x0) {
                *v = b;
            }
        }
    };
    fill(&mut town, 9, 0, 10, 16, PATH);
    fill(&mut town, 1, 9, 18, 9, PATH);
    fill(&mut town, 3, 3, 6, 5, WATER);
    fill(&mut town, 2, 12, 4, 13, FLOWERS);
    fill(&mut town, 14, 11, 17, 13, FLOWERS);
    fill(&mut town, 3, 14, 7, 16, TALL);
    fill(&mut town, 13, 2, 17, 3, TALL);
    fill(&mut town, 1, 7, 7, 7, FENCE);
    fill(&mut town, 16, 6, 17, 7, TREES);
    fill(&mut town, 6, 11, 6, 11, TREES);
    fill(&mut town, 12, 15, 15, 16, TREES);
    town[4][12] = HOUSE_TL;
    town[4][13] = HOUSE_TR;
    town[5][12] = HOUSE_BL;
    town[5][13] = HOUSE_BR;
    fill(&mut town, 12, 6, 13, 6, PATH);
    fill(&mut town, 11, 6, 11, 8, PATH);
    town[8][12] = SIGN;

    let mut out = [0u8; STRIDE * ROWS];
    for (y, row) in town.iter().enumerate() {
        for (x, &b) in row.iter().enumerate() {
            out[(y + PAD) * STRIDE + x + PAD] = b;
        }
    }
    // North connection: the road out of town, as `FillNorthConnectionStrip`
    // would copy it from the connected map.
    for y in 0..PAD {
        for x in 0..W {
            let b = match x {
                9 | 10 => PATH,
                7 | 8 | 11 | 12 => {
                    if y == 1 {
                        TALL
                    } else {
                        GRASS
                    }
                }
                _ => TREES,
            };
            out[y * STRIDE + x + PAD] = b;
        }
    }
    out
}
