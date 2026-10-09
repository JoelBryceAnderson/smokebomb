//! Pokémon: names, types, pictures and palettes from the ROM, the party and
//! the Pokédex from RAM.
//!
//! Layouts from pret/pokecrystal: `constants/pokemon_data_constants.asm`
//! (base data, party structs), `engine/gfx/load_pics.asm` (which picture,
//! `FixPicBank`, `PadFrontpic`), `home/decompress.asm` (the LZ format),
//! `data/pokemon/palettes.asm`.

use crate::mem::{GbMem, Sym};

use super::syms;

pub const NUM_SPECIES: u8 = 251;
pub const UNOWN: u8 = 201;
/// `EGG` in `wPartySpecies` (the struct keeps the species inside).
pub const EGG: u8 = 0xFD;
pub const PARTY_LENGTH: u8 = 6;
/// The `'@'` string terminator.
pub const TERMINATOR: u8 = 0x50;

/// `BASE_DATA_SIZE` and the fields used.
const BASE_SIZE: u32 = 32;
const BASE_TYPE_1: u32 = 7;
const BASE_PIC_SIZE: u32 = 17;
/// `MON_NAME_LENGTH - 1`: `PokemonNames` entries aren't terminated.
const SPECIES_NAME: u32 = 10;
/// `MON_NAME_LENGTH`: a nickname and its terminator.
const NICKNAME: u16 = 11;

/// `party_struct` (`PARTYMON_STRUCT_LENGTH`) and the fields used.
pub mod party {
    pub const LENGTH: u16 = 48;
    pub const SPECIES: u16 = 0;
    pub const DVS: u16 = 21;
    pub const LEVEL: u16 = 31;
    pub const STATUS: u16 = 32;
    /// Big-endian, as all of Crystal's 16-bit stats.
    pub const HP: u16 = 34;
    pub const MAX_HP: u16 = 36;
}

/// `dba_pic` stores a picture's bank less this (`FixPicBank`).
const PICS_FIX: u8 = 0x36;
/// `FixPicBank.PicsBanks` has 23 entries ("Pics 1" to "Pics 23").
const PICS_BANKS: u8 = 23;

pub fn is_species(s: u8) -> bool {
    (1..=NUM_SPECIES).contains(&s)
}

fn rom<M: GbMem + ?Sized>(m: &M, s: Sym, at: u32) -> u8 {
    m.rom(s.rom_offset() + at)
}

/// Up to 10 charmap codes (a name, a type), without the terminator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub codes: [u8; 10],
    pub len: u8,
}

impl Text {
    pub fn new(codes: &[u8]) -> Text {
        let mut t = Text::default();
        for &c in codes.iter().take(10) {
            if c == TERMINATOR {
                break;
            }
            t.codes[t.len as usize] = c;
            t.len += 1;
        }
        t
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.codes[..self.len as usize]
    }

    fn read(f: impl Fn(u32) -> u8, max: u32) -> Text {
        let mut buf = [TERMINATOR; 10];
        for (i, b) in buf.iter_mut().enumerate().take(max as usize) {
            *b = f(i as u32);
        }
        Text::new(&buf)
    }

    /// A number, right-aligned in `width` digits with leading spaces.
    pub fn number(mut n: u16, width: u8) -> Text {
        let mut t = Text {
            codes: [SPACE; 10],
            len: width.min(10),
        };
        for i in (0..t.len as usize).rev() {
            t.codes[i] = DIGIT_0 + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        t
    }

    /// `self` then `other`.
    pub fn then(mut self, other: &[u8]) -> Text {
        for &c in other {
            if self.len as usize == self.codes.len() {
                break;
            }
            self.codes[self.len as usize] = c;
            self.len += 1;
        }
        self
    }
}

const SPACE: u8 = super::charmap::SPACE;
const DIGIT_0: u8 = 0xF6;

/// ASCII capitals, digits and a few marks to charmap codes, for the cube's
/// own labels ("CANCEL", "No.", "SEEN").
pub const fn code(c: u8) -> u8 {
    match c {
        b'A'..=b'Z' => 0x80 + (c - b'A'),
        b'a'..=b'z' => 0xA0 + (c - b'a'),
        b'0'..=b'9' => DIGIT_0 + (c - b'0'),
        b'.' => 0xE8,
        b'/' => 0xF3,
        b'-' => 0xE3,
        b'?' => 0xE6,
        b'!' => 0xE7,
        b':' => 0x9C,
        _ => SPACE,
    }
}

/// [`code`] for a whole string.
pub fn text(s: &str) -> Text {
    let mut t = Text::default();
    for (slot, &c) in t.codes.iter_mut().zip(s.as_bytes()) {
        *slot = code(c);
        t.len += 1;
    }
    t
}

/// A species' name from `PokemonNames`.
pub fn species_name<M: GbMem + ?Sized>(m: &M, species: u8) -> Text {
    let base = (species as u32 - 1) * SPECIES_NAME;
    Text::read(|i| rom(m, syms::POKEMON_NAMES, base + i), SPECIES_NAME)
}

/// Its two types (the same twice for a single type).
pub fn types<M: GbMem + ?Sized>(m: &M, species: u8) -> (u8, u8) {
    let base = (species as u32 - 1) * BASE_SIZE + BASE_TYPE_1;
    (rom(m, syms::BASE_DATA, base), rom(m, syms::BASE_DATA, base + 1))
}

/// A type's name, through `TypeNames` (pointers in the same bank).
pub fn type_name<M: GbMem + ?Sized>(m: &M, ty: u8) -> Text {
    let at = ty as u32 * 2;
    let ptr = rom(m, syms::TYPE_NAMES, at) as u16 | (rom(m, syms::TYPE_NAMES, at + 1) as u16) << 8;
    if !(0x4000..0x8000).contains(&ptr) {
        return Text::default();
    }
    let s = Sym::new(syms::TYPE_NAMES.bank, ptr);
    Text::read(|i| rom(m, s, i), 10)
}

/// The picture's palette: white, the species' two colours, black, as
/// RGB565 (`PokemonPalettes`: two BGR555 colours, normal then shiny).
pub fn palette<M: GbMem + ?Sized>(m: &M, species: u8, shiny: bool) -> [u16; 4] {
    let at = species as u32 * 8 + if shiny { 4 } else { 0 };
    let c = |k: u32| {
        let lo = rom(m, syms::POKEMON_PALETTES, at + k * 2);
        let hi = rom(m, syms::POKEMON_PALETTES, at + k * 2 + 1);
        crate::color::bgr555_to_rgb565(u16::from_le_bytes([lo, hi]))
    };
    [0xFFFF, c(0), c(1), 0x0000]
}

/// Shiny DVs: defence, speed and special 10, attack 2, 3, 6, 7, 10, 11, 14
/// or 15 (`CheckShininess`). `dvs` as stored: attack/defence then
/// speed/special nibbles.
pub fn is_shiny(dvs: [u8; 2]) -> bool {
    let (atk, def, spd, spc) = (dvs[0] >> 4, dvs[0] & 15, dvs[1] >> 4, dvs[1] & 15);
    def == 10 && spd == 10 && spc == 10 && atk & 2 != 0
}

/// Unown's letter, 1–26, from its DVs (`GetUnownLetter`).
pub fn unown_letter(dvs: [u8; 2]) -> u8 {
    let v = ((dvs[0] & 0x60) << 1) | ((dvs[0] & 0x06) << 3) | ((dvs[1] & 0x60) >> 3) | ((dvs[1] & 0x06) >> 1);
    v / (0xFF / 26 + 1) + 1
}

/// A front picture, 5×5 to 7×7 tiles in the game's column-major order.
#[derive(Clone)]
pub struct Pic {
    /// Tiles a side.
    pub size: u8,
    pub tiles: [u8; 7 * 7 * 16],
}

impl Pic {
    pub const SIDE: usize = 56;

    /// Colour 0–3 at (`x`, `y`) of the 56×56 box, placed as `PadFrontpic`
    /// does: a 6×6 picture one tile in from the left and top, a 5×5 one
    /// tile in from the left and two from the top (sitting on the bottom).
    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        let n = self.size as usize;
        let (ox, oy) = match n {
            7 => (0, 0),
            6 => (1, 1),
            _ => (1, 2),
        };
        let (tx, ty) = (x / 8, y / 8);
        if tx < ox || ty < oy || tx - ox >= n || ty - oy >= n {
            return 0;
        }
        let t = (tx - ox) * n + (ty - oy);
        let row = t * 16 + (y % 8) * 2;
        let bit = 7 - (x % 8);
        ((self.tiles[row] >> bit) & 1) | (((self.tiles[row + 1] >> bit) & 1) << 1)
    }
}

/// The front picture of `species` (Unown as `letter`, 1–26), decompressed
/// from the ROM. Only the first frame: the animation frames that follow it
/// in the same stream are left undecompressed.
pub fn front_pic<M: GbMem + ?Sized>(m: &M, species: u8, letter: u8) -> Option<Pic> {
    if !is_species(species) {
        return None;
    }
    let (table, index) = if species == UNOWN {
        (syms::UNOWN_PIC_POINTERS, letter.clamp(1, 26))
    } else {
        (syms::POKEMON_PIC_POINTERS, species)
    };
    let at = (index as u32 - 1) * 6;
    let stored = rom(m, table, at);
    let addr = rom(m, table, at + 1) as u16 | (rom(m, table, at + 2) as u16) << 8;
    // `FixPicBank`: the stored byte, less "Pics 1"'s bank less PICS_FIX,
    // indexes the table of the real banks ("Pics 1" is its first entry).
    let pics1 = rom(m, syms::FIX_PIC_BANK_PICS_BANKS, 0);
    let i = stored.wrapping_sub(pics1.wrapping_sub(PICS_FIX));
    if i >= PICS_BANKS || !(0x4000..0x8000).contains(&addr) {
        return None;
    }
    let bank = rom(m, syms::FIX_PIC_BANK_PICS_BANKS, i as u32);
    let size = rom(
        m,
        syms::BASE_DATA,
        (species as u32 - 1) * BASE_SIZE + BASE_PIC_SIZE,
    ) & 0x0F;
    if !(5..=7).contains(&size) {
        return None;
    }
    let mut pic = Pic {
        size,
        tiles: [0; 7 * 7 * 16],
    };
    let n = size as usize * size as usize * 16;
    let start = Sym::new(bank, addr).rom_offset();
    decompress(|i| m.rom(start + i), &mut pic.tiles[..n]);
    Some(pic)
}

/// Crystal's LZ (`Decompress`, "lz3") from `input` into `out`, stopping at
/// `$FF` or when `out` is full. Returns the bytes written. Copies that
/// reach outside what's been written read 0, and a stream that never ends
/// stops after 8 KiB of input.
pub fn decompress(input: impl Fn(u32) -> u8, out: &mut [u8]) -> usize {
    const END: u8 = 0xFF;
    const LITERAL: u8 = 0;
    const ITERATE: u8 = 1;
    const ALTERNATE: u8 = 2;
    const ZERO: u8 = 3;
    const FLIP: u8 = 5;
    const REVERSE: u8 = 6;
    const LONG: u8 = 7;
    let mut i = 0u32;
    let mut o = 0usize;
    let mut next = || {
        if i >= 8192 {
            return END;
        }
        let b = input(i);
        i += 1;
        b
    };
    loop {
        if o >= out.len() {
            return o;
        }
        let c = next();
        if c == END {
            return o;
        }
        let (cmd, len) = if c >> 5 == LONG {
            let lo = next();
            ((c >> 2) & 7, (((c & 3) as usize) << 8 | lo as usize) + 1)
        } else {
            (c >> 5, (c & 0x1F) as usize + 1)
        };
        match cmd {
            LITERAL => {
                for _ in 0..len {
                    let b = next();
                    if o < out.len() {
                        out[o] = b;
                        o += 1;
                    }
                }
            }
            ITERATE | ALTERNATE | ZERO => {
                let pair = match cmd {
                    ITERATE => {
                        let b = next();
                        [b, b]
                    }
                    ALTERNATE => [next(), next()],
                    _ => [0, 0],
                };
                for k in 0..len {
                    if o < out.len() {
                        out[o] = pair[k % 2];
                        o += 1;
                    }
                }
            }
            _ => {
                // REPEAT (4), FLIP, REVERSE (and LONG inside LONG, which the
                // game treats as REPEAT): copy from earlier output, at a
                // 7-bit distance back (bit 7 set) or a 15-bit offset from
                // the start.
                let a = next();
                let from = if a & 0x80 != 0 {
                    o as isize - (a & 0x7F) as isize - 1
                } else {
                    ((a as isize) << 8) | next() as isize
                };
                for k in 0..len as isize {
                    let src = if cmd == REVERSE { from - k } else { from + k };
                    let b = if (0..o as isize).contains(&src) {
                        out[src as usize]
                    } else {
                        0
                    };
                    let b = if cmd == FLIP { b.reverse_bits() } else { b };
                    if o < out.len() {
                        out[o] = b;
                        o += 1;
                    }
                }
            }
        }
    }
}

/// One party member.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyMon {
    pub species: u8,
    pub egg: bool,
    pub nickname: Text,
    pub level: u8,
    pub status: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub dvs: [u8; 2],
}

impl PartyMon {
    pub fn shiny(&self) -> bool {
        is_shiny(self.dvs)
    }

    pub fn letter(&self) -> u8 {
        unown_letter(self.dvs)
    }
}

/// `wPartyCount`, if it's a party's size.
pub fn party_count<M: GbMem + ?Sized>(m: &M) -> Option<u8> {
    let n = m.byte(syms::W_PARTY_COUNT);
    (1..=PARTY_LENGTH).contains(&n).then_some(n)
}

pub fn party_mon<M: GbMem + ?Sized>(m: &M, i: u8) -> PartyMon {
    let s = syms::W_PARTY_MON1.offset(i as u16 * party::LENGTH);
    let word = |o: u16| (m.byte(s.offset(o)) as u16) << 8 | m.byte(s.offset(o + 1)) as u16;
    let nick = syms::W_PARTY_MON_NICKNAMES.offset(i as u16 * NICKNAME);
    PartyMon {
        species: m.byte(s.offset(party::SPECIES)),
        egg: m.byte(syms::W_PARTY_SPECIES.offset(i as u16)) == EGG,
        nickname: Text::read(|k| m.byte(nick.offset(k as u16)), 10),
        level: m.byte(s.offset(party::LEVEL)),
        status: m.byte(s.offset(party::STATUS)),
        hp: word(party::HP),
        max_hp: word(party::MAX_HP),
        dvs: [m.byte(s.offset(party::DVS)), m.byte(s.offset(party::DVS + 1))],
    }
}

/// A status condition's short name (`PlacePartyMonStatus`'s strings).
pub fn status_name(status: u8) -> Option<&'static str> {
    Some(if status & 7 != 0 {
        "SLP"
    } else if status & 1 << 3 != 0 {
        "PSN"
    } else if status & 1 << 4 != 0 {
        "BRN"
    } else if status & 1 << 5 != 0 {
        "FRZ"
    } else if status & 1 << 6 != 0 {
        "PAR"
    } else {
        return None;
    })
}

fn flag<M: GbMem + ?Sized>(m: &M, table: Sym, species: u8) -> bool {
    if !is_species(species) {
        return false;
    }
    let i = species as u16 - 1;
    m.byte(table.offset(i / 8)) & (1 << (i % 8)) != 0
}

pub fn seen<M: GbMem + ?Sized>(m: &M, species: u8) -> bool {
    flag(m, syms::W_POKEDEX_SEEN, species)
}

pub fn caught<M: GbMem + ?Sized>(m: &M, species: u8) -> bool {
    flag(m, syms::W_POKEDEX_CAUGHT, species)
}

/// Species flagged in a table (`CountSetBits`).
fn count<M: GbMem + ?Sized>(m: &M, table: Sym) -> u16 {
    (0..32u16)
        .map(|i| m.byte(table.offset(i)).count_ones() as u16)
        .sum()
}

pub fn seen_count<M: GbMem + ?Sized>(m: &M) -> u16 {
    count(m, syms::W_POKEDEX_SEEN)
}

pub fn caught_count<M: GbMem + ?Sized>(m: &M) -> u16 {
    count(m, syms::W_POKEDEX_CAUGHT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(stream: &[u8], n: usize) -> std::vec::Vec<u8> {
        let mut out = std::vec![0xAAu8; n];
        let k = decompress(|i| stream.get(i as usize).copied().unwrap_or(0xFF), &mut out);
        out.truncate(k);
        out
    }

    #[test]
    fn every_lz_command() {
        // literal 3, iterate 4, alternate 5, zero 2.
        let s = [0x02, 1, 2, 3, 0x23, 9, 0x44, 7, 8, 0x61, 0xFF];
        assert_eq!(run(&s, 64), [1, 2, 3, 9, 9, 9, 9, 7, 8, 7, 8, 7, 0, 0]);
        // Repeat 3 from offset 1 (running into its own output), flip 2
        // from 1 back, reverse 3 from offset 2.
        let s = [
            0x02, 0x01, 0x80, 0x0F, 0x82, 0x00, 0x01, 0xA1, 0x80, 0xC2, 0x00, 0x02, 0xFF,
        ];
        let out = run(&s, 64);
        assert_eq!(&out[..3], &[0x01, 0x80, 0x0F]);
        assert_eq!(&out[3..6], &[0x80, 0x0F, 0x80]);
        assert_eq!(&out[6..8], &[0x01, 0x80]);
        assert_eq!(&out[8..11], &[0x0F, 0x80, 0x01]);
    }

    #[test]
    fn long_counts_and_a_full_buffer() {
        // LONG: iterate (1) with a 10-bit count of 300.
        let s = [0xE4 | 0x01, (300 - 1 - 256) as u8, 5, 0xFF];
        let out = run(&s, 400);
        assert_eq!(out.len(), 300);
        assert!(out.iter().all(|&b| b == 5));
        // Stops when the output is full, mid-command.
        assert_eq!(run(&s, 10).len(), 10);
    }

    #[test]
    fn numbers_and_labels() {
        assert_eq!(Text::number(7, 3).as_slice(), &[SPACE, SPACE, DIGIT_0 + 7]);
        assert_eq!(
            Text::number(152, 3).as_slice(),
            &[DIGIT_0 + 1, DIGIT_0 + 5, DIGIT_0 + 2]
        );
        assert_eq!(text("No.").as_slice(), &[0x8D, 0xAE, 0xE8]);
    }

    #[test]
    fn shiny_and_unown() {
        assert!(is_shiny([0xAA, 0xAA]));
        assert!(!is_shiny([0x8A, 0xAA]));
        // All the middle bits clear: A. All set: 255 / 10 + 1 = 26.
        assert_eq!(unown_letter([0, 0]), 1);
        assert_eq!(unown_letter([0x66, 0x66]), 26);
    }
}
