//! The demo cart's creatures (original, not Pokémon), laid out in ROM and
//! RAM the way Crystal lays out its Pokémon: base data, names, type names,
//! palettes, LZ-compressed front pictures behind `PokemonPicPointers` and
//! `FixPicBank`, a party and Pokédex flags.

use gbc_cube_core::crystal::mons::party;
use gbc_cube_core::crystal::syms;
use gbc_cube_core::mem::Sym;

use crate::art::bgr;

/// One species: name, types (Crystal's type values), picture size in
/// tiles, body shape, its two middle colours and its shiny ones.
pub struct Species {
    pub name: &'static str,
    pub types: (u8, u8),
    pub size: u8,
    pub shape: u8,
    pub colours: [(u8, u8, u8); 2],
    pub shiny: [(u8, u8, u8); 2],
}

/// Crystal's type values (`constants/type_constants.asm`).
mod ty {
    pub const NORMAL: u8 = 0;
    pub const FLYING: u8 = 2;
    pub const POISON: u8 = 3;
    pub const GROUND: u8 = 4;
    pub const ROCK: u8 = 5;
    pub const FIRE: u8 = 20;
    pub const WATER: u8 = 21;
    pub const GRASS: u8 = 22;
    pub const ELECTRIC: u8 = 23;
    pub const ICE: u8 = 25;
    pub const GHOST: u8 = 8;
}

const TYPE_NAMES: [(u8, &str); 11] = [
    (ty::NORMAL, "NORMAL"),
    (ty::FLYING, "FLYING"),
    (ty::POISON, "POISON"),
    (ty::GROUND, "GROUND"),
    (ty::ROCK, "ROCK"),
    (ty::GHOST, "GHOST"),
    (ty::FIRE, "FIRE"),
    (ty::WATER, "WATER"),
    (ty::GRASS, "GRASS"),
    (ty::ELECTRIC, "ELECTRIC"),
    (ty::ICE, "ICE"),
];

/// Species 1–9. The last three are in the Pokédex unseen.
pub const SPECIES: [Species; 9] = [
    Species {
        name: "BLOB",
        types: (ty::NORMAL, ty::NORMAL),
        size: 6,
        shape: 0,
        colours: [(248, 160, 144), (208, 56, 56)],
        shiny: [(248, 216, 144), (200, 136, 48)],
    },
    Species {
        name: "SPROUT",
        types: (ty::GRASS, ty::POISON),
        size: 5,
        shape: 1,
        colours: [(168, 232, 120), (56, 152, 72)],
        shiny: [(232, 232, 120), (168, 152, 56)],
    },
    Species {
        name: "EMBER",
        types: (ty::FIRE, ty::FIRE),
        size: 6,
        shape: 2,
        colours: [(248, 200, 96), (232, 96, 32)],
        shiny: [(248, 200, 232), (176, 64, 160)],
    },
    Species {
        name: "DRIP",
        types: (ty::WATER, ty::WATER),
        size: 5,
        shape: 3,
        colours: [(160, 208, 248), (48, 104, 216)],
        shiny: [(200, 168, 248), (112, 72, 200)],
    },
    Species {
        name: "ZAP",
        types: (ty::ELECTRIC, ty::FLYING),
        size: 7,
        shape: 4,
        colours: [(248, 232, 104), (200, 160, 24)],
        shiny: [(168, 248, 248), (32, 168, 184)],
    },
    Species {
        name: "PEBBLE",
        types: (ty::ROCK, ty::GROUND),
        size: 7,
        shape: 5,
        colours: [(200, 192, 176), (120, 104, 96)],
        shiny: [(216, 176, 168), (152, 88, 88)],
    },
    Species {
        name: "SHADE",
        types: (ty::GHOST, ty::GHOST),
        size: 6,
        shape: 3,
        colours: [(184, 152, 216), (88, 56, 136)],
        shiny: [(184, 152, 216), (88, 56, 136)],
    },
    Species {
        name: "FROST",
        types: (ty::ICE, ty::ICE),
        size: 6,
        shape: 4,
        colours: [(216, 240, 248), (120, 176, 216)],
        shiny: [(216, 240, 248), (120, 176, 216)],
    },
    Species {
        name: "GALE",
        types: (ty::FLYING, ty::FLYING),
        size: 5,
        shape: 1,
        colours: [(232, 232, 232), (160, 168, 184)],
        shiny: [(232, 232, 232), (160, 168, 184)],
    },
];

/// A party member: species (1-based), nickname, level, HP, status, DVs,
/// and whether it's still an egg.
pub struct Member {
    pub species: u8,
    pub nickname: &'static str,
    pub level: u8,
    pub hp: (u16, u16),
    pub status: u8,
    pub dvs: [u8; 2],
    pub egg: bool,
}

pub const PARTY: [Member; 5] = [
    Member {
        species: 1,
        nickname: "CUBE",
        level: 7,
        hp: (23, 23),
        status: 0,
        dvs: [0x98, 0x76],
        egg: false,
    },
    Member {
        species: 2,
        nickname: "SPROUT",
        level: 5,
        hp: (9, 19),
        status: 1 << 3,
        dvs: [0x45, 0x67],
        egg: false,
    },
    Member {
        species: 3,
        nickname: "EMBER",
        level: 9,
        hp: (3, 28),
        status: 0,
        dvs: [0x12, 0x34],
        egg: false,
    },
    Member {
        species: 5,
        nickname: "SPARKY",
        level: 12,
        hp: (31, 34),
        status: 0,
        // Shiny.
        dvs: [0xAA, 0xAA],
        egg: false,
    },
    Member {
        species: 4,
        nickname: "EGG",
        level: 1,
        hp: (0, 0),
        status: 0,
        dvs: [0, 0],
        egg: true,
    },
];

/// Seen and caught species.
pub const SEEN: [u8; 6] = [1, 2, 3, 4, 5, 6];
pub const CAUGHT: [u8; 4] = [1, 2, 3, 5];

/// Where this cart puts what Crystal finds through its tables.
const PICS_BANK: u8 = 0x50;
const TYPE_STRINGS: u16 = 0x6000;

/// Colour 0–3 of a creature's picture at (`x`, `y`) of a `side`-pixel box:
/// a body, a feature on top by `shape` (a leaf, a flame, a drop's tip,
/// ears, a rock's corners), outlined, with eyes and a shine.
pub fn creature(shape: u8, x: usize, y: usize, side: usize) -> u8 {
    let inside = |x: i32, y: i32| -> Option<bool> {
        if x < 0 || y < 0 || x >= side as i32 || y >= side as i32 {
            return None;
        }
        let s = side as f32;
        let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
        let (dx, dy) = (u - 0.5, v - 0.62);
        let body = match shape {
            5 => dx.abs().max(dy.abs()) <= 0.36 && dx.abs() + dy.abs() <= 0.5,
            _ => (dx / 0.42).powi(2) + (dy / 0.34).powi(2) <= 1.0,
        };
        let feature = match shape {
            1 => {
                ((u - 0.64) / 0.17).powi(2) + ((v - 0.2) / 0.07).powi(2) <= 1.0
                    || (dx.abs() < 0.03 && v > 0.2)
            }
            2 => (0.06..0.32).contains(&v) && dx.abs() < (v - 0.06) * 0.7,
            3 => (0.08..0.32).contains(&v) && dx.abs() < (v - 0.08) * 1.1,
            4 => {
                (0.06..0.34).contains(&v)
                    && ((u - 0.28).abs() < (v - 0.06) * 0.45 || (u - 0.72).abs() < (v - 0.06) * 0.45)
            }
            _ => false,
        };
        Some(body || (feature && v < 0.4))
    };
    let (xi, yi) = (x as i32, y as i32);
    if inside(xi, yi) != Some(true) {
        return 0;
    }
    let edge = [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .iter()
        .any(|(a, b)| inside(xi + a, yi + b) != Some(true));
    let s = side as f32;
    let (u, v) = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
    let eye = |ex: f32| ((u - ex) / 0.05).powi(2) + ((v - 0.56) / 0.07).powi(2) <= 1.0;
    if edge || eye(0.38) || eye(0.62) {
        return 3;
    }
    if v < 0.4 && shape != 0 && shape != 5 && shape != 3 {
        return 1; // the feature, light
    }
    if (u - 0.36).powi(2) + (v - 0.44).powi(2) < 0.006 {
        return 1; // shine
    }
    2
}

/// A creature's picture as Crystal stores one: 2bpp tiles, column by
/// column.
pub fn pic_tiles(s: &Species) -> Vec<u8> {
    let n = s.size as usize;
    let side = n * 8;
    let mut out = Vec::with_capacity(n * n * 16);
    for tx in 0..n {
        for ty in 0..n {
            for r in 0..8 {
                let (mut lo, mut hi) = (0u8, 0u8);
                for i in 0..8 {
                    let c = creature(s.shape, tx * 8 + i, ty * 8 + r, side);
                    lo |= (c & 1) << (7 - i);
                    hi |= ((c >> 1) & 1) << (7 - i);
                }
                out.push(lo);
                out.push(hi);
            }
        }
    }
    out
}

/// Crystal's LZ (see `gbc_cube_core::crystal::mons::decompress`), greedy:
/// runs of zeros, runs of a byte, copies from up to 128 bytes back, and
/// literals.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let flush = |out: &mut Vec<u8>, lit: &mut Vec<u8>| {
        for chunk in lit.chunks(32) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        lit.clear();
    };
    let mut i = 0;
    while i < data.len() {
        let run = |b: u8| data[i..].iter().take(32).take_while(|&&x| x == b).count();
        let same = run(data[i]);
        let (mut best, mut dist) = (0, 0);
        for d in 1..=i.min(128) {
            let n = (0..32.min(data.len() - i))
                .take_while(|&k| data[i + k] == data[i + k - d])
                .count();
            if n > best {
                (best, dist) = (n, d);
            }
        }
        if data[i] == 0 && same >= 2 && same >= best {
            flush(&mut out, &mut lit);
            out.push(0x60 | (same - 1) as u8);
            i += same;
        } else if best >= 3 && best >= same {
            flush(&mut out, &mut lit);
            out.push(0x80 | (best - 1) as u8);
            out.push(0x80 | (dist - 1) as u8);
            i += best;
        } else if same >= 3 {
            flush(&mut out, &mut lit);
            out.push(0x20 | (same - 1) as u8);
            out.push(data[i]);
            i += same;
        } else {
            lit.push(data[i]);
            i += 1;
        }
    }
    flush(&mut out, &mut lit);
    out.push(0xFF);
    out
}

fn put(rom: &mut [u8], s: Sym, at: usize, bytes: &[u8]) {
    let o = s.rom_offset() as usize + at;
    rom[o..o + bytes.len()].copy_from_slice(bytes);
}

/// ASCII to Crystal's charmap, for the cart's names.
pub fn codes(s: &str) -> Vec<u8> {
    s.bytes().map(gbc_cube_core::crystal::mons::code).collect()
}

/// Base data, names, type names, palettes and pictures into the ROM.
pub fn write_rom(rom: &mut [u8]) {
    let mut pic_at = 0x4000usize;
    for (k, s) in SPECIES.iter().enumerate() {
        let sp = k + 1;
        let mut base = [0u8; 32];
        base[0] = sp as u8;
        base[7] = s.types.0;
        base[8] = s.types.1;
        base[17] = s.size * 0x11;
        put(rom, syms::BASE_DATA, k * 32, &base);
        let mut name = [0x50u8; 10];
        for (slot, c) in name.iter_mut().zip(codes(s.name)) {
            *slot = c;
        }
        put(rom, syms::POKEMON_NAMES, k * 10, &name);
        let mut pal = Vec::new();
        for (r, g, b) in s.colours.iter().chain(&s.shiny) {
            pal.extend_from_slice(&bgr(*r, *g, *b).to_le_bytes());
        }
        put(rom, syms::POKEMON_PALETTES, sp * 8, &pal);
        // The picture, and its pointer: bank less PICS_FIX (`dba_pic`).
        let lz = compress(&pic_tiles(s));
        put(rom, Sym::new(PICS_BANK, pic_at as u16), 0, &lz);
        let mut ptr = vec![PICS_BANK - 0x36];
        ptr.extend_from_slice(&(pic_at as u16).to_le_bytes());
        put(rom, syms::POKEMON_PIC_POINTERS, k * 6, &ptr);
        pic_at += lz.len();
        assert!(pic_at < 0x8000, "pictures overflow their bank");
    }
    // `FixPicBank.PicsBanks`: "Pics 1" is this cart's PICS_BANK.
    let banks: Vec<u8> = (0..23).map(|k| PICS_BANK + k).collect();
    put(rom, syms::FIX_PIC_BANK_PICS_BANKS, 0, &banks);
    // Type names: a pointer per type value into the same bank.
    let mut at = TYPE_STRINGS;
    for (t, name) in TYPE_NAMES {
        put(rom, syms::TYPE_NAMES, t as usize * 2, &at.to_le_bytes());
        let mut s = codes(name);
        s.push(0x50);
        put(rom, Sym::new(syms::TYPE_NAMES.bank, at), 0, &s);
        at += s.len() as u16;
    }
}

/// The party and the Pokédex flags into WRAM.
pub fn write_ram(wram: &mut [u8]) {
    let at = |s: Sym| match s.addr {
        0xC000..=0xCFFF => s.addr as usize - 0xC000,
        _ => s.bank.max(1) as usize * 0x1000 + s.addr as usize - 0xD000,
    };
    wram[at(syms::W_PARTY_COUNT)] = PARTY.len() as u8;
    for (i, m) in PARTY.iter().enumerate() {
        wram[at(syms::W_PARTY_SPECIES) + i] = if m.egg { 0xFD } else { m.species };
        let s = at(syms::W_PARTY_MON1) + i * party::LENGTH as usize;
        wram[s..s + party::LENGTH as usize].fill(0);
        wram[s + party::SPECIES as usize] = m.species;
        wram[s + party::DVS as usize..s + party::DVS as usize + 2].copy_from_slice(&m.dvs);
        wram[s + party::LEVEL as usize] = m.level;
        wram[s + party::STATUS as usize] = m.status;
        wram[s + party::HP as usize..s + party::HP as usize + 2].copy_from_slice(&m.hp.0.to_be_bytes());
        wram[s + party::MAX_HP as usize..s + party::MAX_HP as usize + 2]
            .copy_from_slice(&m.hp.1.to_be_bytes());
        let n = at(syms::W_PARTY_MON_NICKNAMES) + i * 11;
        wram[n..n + 11].fill(0x50);
        for (k, c) in codes(m.nickname).into_iter().enumerate() {
            wram[n + k] = c;
        }
    }
    wram[at(syms::W_PARTY_SPECIES) + PARTY.len()] = 0xFF;
    for (table, list) in [
        (syms::W_POKEDEX_SEEN, &SEEN[..]),
        (syms::W_POKEDEX_CAUGHT, &CAUGHT[..]),
    ] {
        for &sp in list {
            let i = sp as usize - 1;
            wram[at(table) + i / 8] |= 1 << (i % 8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbc_cube_core::crystal::mons::decompress;

    #[test]
    fn pictures_round_trip_through_crystals_lz() {
        for s in &SPECIES {
            let raw = pic_tiles(s);
            let lz = compress(&raw);
            let mut out = vec![0u8; raw.len()];
            let n = decompress(|i| lz.get(i as usize).copied().unwrap_or(0xFF), &mut out);
            assert_eq!(n, raw.len(), "{}", s.name);
            assert_eq!(out, raw, "{}", s.name);
            assert!(lz.len() < raw.len(), "{} didn't compress", s.name);
        }
    }
}
