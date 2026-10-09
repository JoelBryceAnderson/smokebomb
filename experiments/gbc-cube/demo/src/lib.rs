//! A stand-in for Pokémon Crystal, for working on the cube without a ROM.
//!
//! The repo carries no ROMs, and nobody should need one to see the cube
//! work. This crate builds a tiny Game Boy Color cartridge image in memory
//! (an `ei; halt` loop and some data tables) and then plays the part of the
//! game's code itself, from Rust, between frames: it walks a player round a
//! small town with original art, scrolls the screen the way Crystal does,
//! opens text boxes, a start menu and a battle, and writes every piece of
//! state **at the addresses and in the formats Crystal uses** (the symbols
//! in `gbc_cube_core::crystal::syms`): the map block buffer, the overworld
//! and BG map anchors, `hSCX`/`hSCY`, the object structs, `wTilemap`, the
//! shadow OAM, and in ROM the metatiles, the palette map and the `Facings`
//! table. The emulator's real PPU then draws the frame from VRAM and OAM.
//!
//! So the cube's Crystal code runs unchanged on it, and the world renderer
//! can be checked pixel for pixel against a real PPU frame. What it can't
//! check is whether Crystal really behaves the way this crate imitates it;
//! that takes the real game (see the README).
//!
//! The order of work per frame mirrors Crystal's: after `run_frame` the
//! cube reads RAM (the next frame's state), then [`Demo::step`] does the
//! VBlank handler's copies to the hardware, then the next frame's logic.

pub mod art;
pub mod map;
pub mod mons;

use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::crystal::{obj, syms};
use gbc_cube_core::mem::Sym;
use gbc_cube_emu::Emulator;

use art::{bg, char_tile, Tile};
use map::{block, PAD, ROWS, STRIDE};

/// Where the demo cart keeps its tables in ROM.
pub const METATILES: Sym = Sym::new(0x05, 0x4000);
pub const PALMAP: Sym = Sym::new(gbc_cube_core::crystal::PALMAP_BANK, 0x4100);
const FACING_TEMPLATES: u16 = 0x4100;

/// Object tile layout per character (offsets from its `SPRITE_TILE`).
mod frames {
    pub const DOWN: u8 = 0;
    pub const UP: u8 = 4;
    pub const LEFT: u8 = 8;
    pub const DOWN_WALK: u8 = 12;
    pub const UP_WALK: u8 = 16;
    pub const LEFT_WALK: u8 = 20;
}

/// The demo cartridge image: 2 MiB, MBC5 with 8 KiB of battery RAM, CGB.
pub fn rom() -> Vec<u8> {
    let mut rom = vec![0u8; 2 * 1024 * 1024];
    rom[0x40] = 0xD9; // VBlank: reti
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]); // nop; jp $0150
    let title = b"GBCCUBEDEMO";
    rom[0x134..0x134 + title.len()].copy_from_slice(title);
    rom[0x143] = 0x80; // CGB
    rom[0x147] = 0x1B; // MBC5 + RAM + battery
    rom[0x148] = 0x06; // 2 MiB, Crystal's size: its data banks go up to $48 and past
    rom[0x149] = 0x02; // 8 KiB
                       // ld a, 1; ldh [IE], a; ei; .loop: halt; nop; jr .loop
    rom[0x150..0x159].copy_from_slice(&[0x3E, 0x01, 0xE0, 0xFF, 0xFB, 0x76, 0x00, 0x18, 0xFC]);
    let mut x = 0u8;
    for b in &rom[0x134..=0x14C] {
        x = x.wrapping_sub(*b).wrapping_sub(1);
    }
    rom[0x14D] = x;

    // Metatiles.
    for (i, m) in map::metatiles().iter().enumerate() {
        let o = METATILES.rom_offset() as usize + i * 16;
        rom[o..o + 16].copy_from_slice(m);
    }
    // Palette map: a nibble per tile ID, low nibble for even IDs.
    for (id, _, pal) in art::BG_TILES {
        let bank1 = if id >= 0x80 { 0x08 } else { 0 };
        let o = PALMAP.rom_offset() as usize + id as usize / 2;
        let nib = (pal | bank1) & 0x0F;
        rom[o] |= if id & 1 == 0 { nib } else { nib << 4 };
    }
    // Facings: Crystal's format (count, then y, x, attributes, tile), with
    // this cart's tile layout.
    const REL: u8 = obj::RELATIVE_ATTRIBUTES;
    const XF: u8 = 0x20;
    let upright = |t: u8| {
        [
            (0, 0, 0, t),
            (0, 8, 0, t + 1),
            (8, 0, REL, t + 2),
            (8, 8, REL, t + 3),
        ]
    };
    let mirrored = |t: u8| {
        [
            (0, 8, XF, t),
            (0, 0, XF, t + 1),
            (8, 8, REL | XF, t + 2),
            (8, 0, REL | XF, t + 3),
        ]
    };
    use frames::*;
    let templates: [[(u8, u8, u8, u8); 4]; 16] = [
        upright(DOWN),
        upright(DOWN_WALK),
        upright(DOWN),
        mirrored(DOWN_WALK),
        upright(UP),
        upright(UP_WALK),
        upright(UP),
        mirrored(UP_WALK),
        upright(LEFT),
        upright(LEFT_WALK),
        upright(LEFT),
        upright(LEFT_WALK),
        mirrored(LEFT),
        mirrored(LEFT_WALK),
        mirrored(LEFT),
        mirrored(LEFT_WALK),
    ];
    let bank1 = 0x4000usize;
    let mut at = FACING_TEMPLATES;
    let empty = at;
    rom[bank1 + (at as usize - 0x4000)] = 0;
    at += 1;
    for f in 0..obj::NUM_FACINGS as usize {
        let ptr = if let Some(t) = templates.get(f) {
            let p = at;
            let o = bank1 + (at as usize - 0x4000);
            rom[o] = 4;
            for (k, e) in t.iter().enumerate() {
                rom[o + 1 + 4 * k..o + 5 + 4 * k].copy_from_slice(&[e.0, e.1, e.2, e.3]);
            }
            at += 17;
            p
        } else {
            empty
        };
        let t = bank1 + (syms::FACINGS.addr as usize - 0x4000) + 2 * f;
        rom[t..t + 2].copy_from_slice(&ptr.to_le_bytes());
    }
    mons::write_rom(&mut rom);
    rom
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Down,
    Up,
    Left,
    Right,
}

impl Dir {
    fn step(self) -> (i32, i32) {
        match self {
            Dir::Down => (0, 1),
            Dir::Up => (0, -1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }

    fn from_buttons(b: Buttons) -> Option<Dir> {
        if b.contains(Buttons::UP) {
            Some(Dir::Up)
        } else if b.contains(Buttons::DOWN) {
            Some(Dir::Down)
        } else if b.contains(Buttons::LEFT) {
            Some(Dir::Left)
        } else if b.contains(Buttons::RIGHT) {
            Some(Dir::Right)
        } else {
            None
        }
    }
}

/// Someone on the map, in 16 px steps.
#[derive(Clone, Copy, Debug)]
struct Mover {
    x: i32,
    y: i32,
    facing: Dir,
    /// Step in progress and how many pixels of it are done.
    moving: Option<(Dir, i32)>,
    parity: bool,
}

impl Mover {
    fn new(x: i32, y: i32) -> Self {
        Mover {
            x,
            y,
            facing: Dir::Down,
            moving: None,
            parity: false,
        }
    }

    /// Map pixel of its 16×16 cell's top-left (buffer coordinates).
    fn px(&self) -> (i32, i32) {
        let (mut x, mut y) = (PAD as i32 * 32 + self.x * 16, PAD as i32 * 32 + self.y * 16);
        if let Some((d, t)) = self.moving {
            let (dx, dy) = d.step();
            x += dx * t;
            y += dy * t;
        }
        (x, y)
    }

    /// Crystal's FACING_* value: four per direction, odd ones the walking
    /// frames, 1 and 3 alternating feet.
    fn facing_value(&self) -> u8 {
        let base = match self.facing {
            Dir::Down => 0,
            Dir::Up => 4,
            Dir::Left => 8,
            Dir::Right => 12,
        };
        match self.moving {
            Some((_, t)) if (4..12).contains(&t) => base + if self.parity { 3 } else { 1 },
            _ => base,
        }
    }

    fn advance(&mut self) {
        if let Some((d, t)) = self.moving {
            if t + 1 >= 16 {
                let (dx, dy) = d.step();
                self.x += dx;
                self.y += dy;
                self.moving = None;
                self.parity = !self.parity;
            } else {
                self.moving = Some((d, t + 1));
            }
        }
    }

    fn target(&self) -> Option<(i32, i32)> {
        self.moving
            .map(|(d, _)| (self.x + d.step().0, self.y + d.step().1))
    }
}

#[derive(Clone, Debug)]
struct TextBox {
    pages: Vec<[String; 2]>,
    page: usize,
    shown: usize,
    /// What happens when the last page is dismissed.
    then: AfterText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterText {
    Close,
    BattleMenu,
    EndBattle,
    /// The professor's speech ends in the naming screen.
    Naming,
}

#[derive(Clone, Debug)]
enum Mode {
    Walk,
    Text(TextBox),
    Menu(usize),
    Battle {
        cursor: usize,
        text: Option<TextBox>,
    },
    Naming(Naming),
    /// The party screen: the cursor (party size = CANCEL), and the
    /// STATS/CANCEL submenu's cursor when it's open.
    Party {
        cursor: usize,
        sub: Option<usize>,
    },
    /// The Pokédex list, at an index into [`DEX_ORDER`].
    Dex {
        pos: usize,
    },
}

/// The naming screen's state: the cursor's key (column 0–8, row 0–4, row 4
/// being UPPER/DEL/END) and the name so far.
#[derive(Clone, Debug, Default)]
struct Naming {
    col: u8,
    row: u8,
    name: String,
}

/// The start menu: labels and Crystal's `STARTMENUITEM_*` for each (the
/// cube picks icons by those). `None` is the player's name, as Crystal's
/// status item. INTRO stands in for the contest's QUIT.
const MENU_ITEMS: [(Option<&str>, u8); 7] = [
    (Some("MONDEX"), 0),
    (Some("PARTY"), 1),
    (Some("PACK"), 2),
    (None, 3),
    (Some("INTRO"), 8),
    (Some("OPTION"), 5),
    (Some("EXIT"), 6),
];
/// `StartMenu.Items`'s bank and address, which tell the cube the open
/// menu is the start menu.
const START_MENU_BANK: u8 = 0x04;
/// The Pokédex list's species, in order.
const DEX_ORDER: [u8; 9] = [1, 2, 3, 4, 5, 6, 7, 8, 9];
/// Crystal's upper-case keyboard (`NameInputUpper`), less the symbols the
/// demo's font lacks, and its bottom row.
const KEYBOARD: [&str; 4] = ["ABCDEFGHI", "JKLMNOPQR", "STUVWXYZ ", "-?!/.,   "];
const KEYBOARD_CMDS: &str = "UPPER  DEL   END ";
const NAME_LEN: usize = 7;
/// Where the naming screen's cursor's sprite animation struct is.
const CURSOR_STRUCT: u16 = 0xC314;
const SPEECH: &str = "HELLO THERE! WELCOME TO THE WORLD OF THE CUBE! THIS IS A BLOB. IT LIVES ON THE DIE WITH YOU. NOW, WHAT IS YOUR NAME?";
const BATTLE_ITEMS: [&str; 4] = ["FIGHT", "MON", "PACK", "RUN"];

/// The game.
pub struct Demo {
    blocks: [u8; STRIDE * ROWS],
    meta: [[u8; 16]; block::COUNT],
    player: Mover,
    npcs: [(Mover, (i32, i32), u32); 2],
    mode: Mode,
    rng: u32,
    frame: u64,
    prev: Buttons,
    /// VRAM writes for the next VBlank: (offset into VRAM, byte).
    vram_queue: Vec<(usize, u8)>,
    /// Palette writes for the next VBlank: (BG palette byte index, value).
    pal_queue: Vec<(u8, u8)>,
    anchor_tile: (i32, i32),
    /// The last VBlank wrote animated tile graphics.
    animated: bool,
    name: String,
}

impl Default for Demo {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the BG tile set lives: IDs `$00`–`$7F` at `$9000` (signed
/// addressing, LCDC bit 4 clear, as Crystal's overworld).
fn bg_tile_vram(id: u8, bank1: bool) -> usize {
    let base = if id < 0x80 {
        0x1000 + id as usize * 16
    } else {
        0x0800 + (id as usize - 0x80) * 16
    };
    base + if bank1 { 0x2000 } else { 0 }
}

fn encode(t: &Tile) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (r, row) in t.iter().enumerate() {
        for (i, c) in row.bytes().enumerate() {
            let v = c - b'0';
            out[2 * r] |= (v & 1) << (7 - i);
            out[2 * r + 1] |= ((v >> 1) & 1) << (7 - i);
        }
    }
    out
}

fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for w in s.split(' ') {
        let cur = lines.last_mut().unwrap();
        if !cur.is_empty() && cur.len() + 1 + w.len() > width {
            lines.push(w.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(w);
        }
    }
    lines
}

impl TextBox {
    fn new(s: &str, then: AfterText) -> TextBox {
        let lines = wrap_text(s, 17);
        let pages = lines
            .chunks(2)
            .map(|c| [c[0].clone(), c.get(1).cloned().unwrap_or_default()])
            .collect();
        TextBox {
            pages,
            page: 0,
            shown: 0,
            then,
        }
    }

    fn page_len(&self) -> usize {
        let p = &self.pages[self.page];
        p[0].len() + p[1].len()
    }

    fn typed(&self) -> bool {
        self.shown >= self.page_len()
    }
}

impl Demo {
    pub fn new() -> Self {
        Demo {
            blocks: map::blocks(),
            meta: map::metatiles(),
            player: Mover::new(18, 18),
            npcs: [
                (Mover::new(26, 19), (26, 19), 40),
                (Mover::new(31, 25), (31, 25), 70),
            ],
            mode: Mode::Walk,
            rng: 0x1234_5678,
            frame: 0,
            prev: Buttons::NONE,
            vram_queue: Vec::new(),
            pal_queue: Vec::new(),
            anchor_tile: (0, 0),
            animated: false,
            name: String::new(),
        }
    }

    fn rand(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Load the graphics and palettes and set up the first frame. Call once
    /// after creating the emulator with [`rom`].
    pub fn boot(&mut self, emu: &mut Emulator) {
        for (pal_ram, pals) in [(0xFF68u16, &art::BG_PALETTES), (0xFF6A, &art::OBJ_PALETTES)] {
            emu.write(pal_ram, 0x80);
            for c in pals.iter().flatten() {
                emu.write(pal_ram + 1, *c as u8);
                emu.write(pal_ram + 1, (*c >> 8) as u8);
            }
        }
        let m = emu.mem_mut();
        // Map tiles.
        for (id, t, _) in art::BG_TILES {
            let o = bg_tile_vram(id & 0x7F, id >= 0x80);
            m.vram[o..o + 16].copy_from_slice(&encode(t));
        }
        // Text box frame, blank, font.
        for (i, t) in art::FRAME.iter().enumerate() {
            let o = bg_tile_vram(0x79 + i as u8, false);
            m.vram[o..o + 16].copy_from_slice(&encode(t));
        }
        let o = bg_tile_vram(0x7F, false);
        m.vram[o..o + 16].fill(0);
        // The naming screen's border, ■ ($60): a light dotted fill.
        let border: Tile = [
            "11111111", "12121212", "11111111", "21212121", "11111111", "12121212", "11111111", "21212121",
        ];
        let o = bg_tile_vram(0x60, false);
        m.vram[o..o + 16].copy_from_slice(&encode(&border));
        // The Pokédex's divider tiles: a line.
        for id in [0x53, 0x54, 0x59, 0x5A, 0x5B] {
            let o = bg_tile_vram(id, false);
            m.vram[o..o + 16].copy_from_slice(&encode(&art::FRAME[3]));
        }
        mons::write_ram(m.wram);
        let font = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!?.,':/->v"
            .chars()
            .map(|c| (char_tile(c), art::glyph(c)));
        for (code, g) in font.chain(art::LOWER) {
            let o = bg_tile_vram(code, false);
            for (r, bits) in g.iter().enumerate() {
                // Dark text (colour 3) on paper (colour 0), one pixel in.
                m.vram[o + 2 * r] = bits << 2;
                m.vram[o + 2 * r + 1] = bits << 2;
            }
            m.vram[o + 14] = 0;
            m.vram[o + 15] = 0;
        }
        // Battle art: the monster (6×6 tiles from $20) and an HP bar tile.
        for ty in 0..6 {
            for tx in 0..6 {
                let o = bg_tile_vram(0x20 + (ty * 6 + tx) as u8, false);
                for r in 0..8 {
                    let (mut lo, mut hi) = (0u8, 0u8);
                    for i in 0..8 {
                        let c = art::blob(tx * 8 + i, ty * 8 + r);
                        lo |= (c & 1) << (7 - i);
                        hi |= ((c >> 1) & 1) << (7 - i);
                    }
                    m.vram[o + 2 * r] = lo;
                    m.vram[o + 2 * r + 1] = hi;
                }
            }
        }
        let bar: Tile = [
            "00000000", "00000000", "11111111", "22222222", "22222222", "11111111", "00000000", "00000000",
        ];
        let o = bg_tile_vram(0x50, false);
        m.vram[o..o + 16].copy_from_slice(&encode(&bar));
        // Characters: the player and a walker in bank 1, a gardener in bank 0.
        for (bank, base) in [(0x2000usize, 0usize), (0x2000, 24), (0, 0)] {
            for (f, tiles) in art::PERSON.iter().enumerate() {
                for (k, t) in tiles.iter().enumerate() {
                    let o = bank + (base + f * 4 + k) * 16;
                    m.vram[o..o + 16].copy_from_slice(&encode(t));
                }
            }
        }
        // LCD: on, window map $9C00 (window hidden below the screen), BG
        // tiles at $8800/$9000, BG map $9800, 8×8 objects on, BG on.
        m.io[0x40] = 0xE3;
        m.io[0x4A] = 0x90;
        m.io[0x4B] = 0x07;
        self.write_map_state(emu);
        // The first frame's state goes to the hardware; the second is
        // prepared and waiting, as after any `step`.
        self.logic(emu, Buttons::NONE);
        self.vblank(emu);
        self.logic(emu, Buttons::NONE);
    }

    /// Between frames: the VBlank handler's copies, then the next frame's
    /// game logic with this frame's joypad.
    pub fn step(&mut self, emu: &mut Emulator, buttons: Buttons) {
        self.vblank(emu);
        self.logic(emu, buttons);
    }

    pub fn in_battle(&self) -> bool {
        matches!(self.mode, Mode::Battle { .. })
    }

    /// A screen that isn't the map: a battle, the intro, the naming screen.
    fn full_screen(&self) -> bool {
        match &self.mode {
            Mode::Battle { .. } | Mode::Naming(_) | Mode::Party { .. } | Mode::Dex { .. } => true,
            Mode::Text(tb) => tb.then == AfterText::Naming,
            _ => false,
        }
    }

    /// The player's name, once the intro has asked for it.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The last [`Demo::step`]'s VBlank changed tile graphics (the water and
    /// flower animation), as Crystal's VBlank handler does. The cube read
    /// VRAM before that, so animated tiles show their previous frame for
    /// one frame.
    pub fn animated(&self) -> bool {
        self.animated
    }

    /// The player's map position in steps.
    pub fn player(&self) -> (i32, i32) {
        (self.player.x, self.player.y)
    }

    fn vblank(&mut self, emu: &mut Emulator) {
        self.wram(emu, syms::W_V_BLANK_OCCURRED, 0);
        for (i, v) in std::mem::take(&mut self.pal_queue) {
            emu.write(0xFF68, i);
            emu.write(0xFF69, v);
        }
        let m = emu.mem_mut();
        let scx = m.io[(syms::H_SCX.addr - 0xFF00) as usize];
        let scy = m.io[(syms::H_SCY.addr - 0xFF00) as usize];
        m.io[0x43] = scx;
        m.io[0x42] = scy;
        let base = syms::W_SHADOW_OAM.addr as usize - 0xC000;
        m.oam.copy_from_slice(&m.wram[base..base + 0xA0]);
        self.animated = false;
        for (o, v) in self.vram_queue.drain(..) {
            // Tile graphics (not the BG map): only the animation writes them
            // after boot.
            self.animated |= o < 0x1800;
            m.vram[o] = v;
        }
    }

    fn wram(&self, emu: &mut Emulator, s: Sym, v: u8) {
        let m = emu.mem_mut();
        let i = match s.addr {
            0xC000..=0xCFFF => s.addr as usize - 0xC000,
            0xD000..=0xDFFF => s.bank.max(1) as usize * 0x1000 + s.addr as usize - 0xD000,
            0xFF80..=0xFFFF => {
                m.io[s.addr as usize - 0xFF00] = v;
                return;
            }
            _ => panic!("not WRAM/HRAM: {s:?}"),
        };
        m.wram[i] = v;
    }

    fn wram16(&self, emu: &mut Emulator, s: Sym, v: u16) {
        self.wram(emu, s, v as u8);
        self.wram(emu, s.offset(1), (v >> 8) as u8);
    }

    /// The map header fields and the block buffer.
    fn write_map_state(&self, emu: &mut Emulator) {
        self.wram(emu, syms::W_MAP_WIDTH, map::W as u8);
        self.wram(emu, syms::W_MAP_HEIGHT, map::H as u8);
        self.wram(emu, syms::W_MAP_BORDER_BLOCK, block::TREES);
        self.wram(emu, syms::W_MAP_GROUP, 24);
        self.wram(emu, syms::W_MAP_NUMBER, 1);
        self.wram(emu, syms::W_TILESET_BLOCKS_BANK, METATILES.bank);
        self.wram16(emu, syms::W_TILESET_BLOCKS_ADDRESS, METATILES.addr);
        self.wram16(emu, syms::W_TILESET_PALETTES, PALMAP.addr);
        let base = syms::W_OVERWORLD_MAP_BLOCKS.addr as usize - 0xC000;
        let m = emu.mem_mut();
        m.wram[base..base + 1300].fill(0);
        m.wram[base..base + self.blocks.len()].copy_from_slice(&self.blocks);
    }

    /// The tile ID (as the metatile has it) and attributes at map tile
    /// (`tx`, `ty`); the border block where the buffer is 0.
    fn map_tile(&self, tx: i32, ty: i32) -> (u8, u8) {
        let (bx, by) = (tx.div_euclid(4), ty.div_euclid(4));
        let b = if bx < 0 || by < 0 || bx >= STRIDE as i32 || by >= ROWS as i32 {
            block::TREES
        } else {
            match self.blocks[by as usize * STRIDE + bx as usize] {
                0 => block::TREES,
                b => b,
            }
        };
        let raw = self.meta[b as usize][(ty.rem_euclid(4) * 4 + tx.rem_euclid(4)) as usize];
        let pal = art::BG_TILES.iter().find(|t| t.0 == raw).map_or(0, |t| t.2);
        (raw, pal | if raw >= 0x80 { 0x08 } else { 0 })
    }

    fn walkable(&self, x: i32, y: i32) -> bool {
        let ok = [
            bg::GRASS,
            bg::GRASS_B,
            bg::TALL_GRASS,
            bg::PATH,
            bg::PATH_B,
            bg::FLOWER,
        ];
        (0..2).all(|j| {
            (0..2).all(|i| {
                ok.contains(
                    &self
                        .map_tile(x * 2 + i + PAD as i32 * 4, y * 2 + j + PAD as i32 * 4)
                        .0,
                )
            })
        }) && x >= 0
            && y >= 0
            && x < map::W as i32 * 2
            && y < map::H as i32 * 2
    }

    fn occupied(&self, x: i32, y: i32, except: Option<usize>) -> bool {
        let at = |m: &Mover| (m.x, m.y) == (x, y) || m.target() == Some((x, y));
        at(&self.player)
            || self
                .npcs
                .iter()
                .enumerate()
                .any(|(i, n)| Some(i) != except && at(&n.0))
    }

    /// Queue a BG map entry (tile and attributes) for screen tile (`sx`,
    /// `sy`) relative to the current anchor, and put it in `wTilemap` /
    /// `wAttrmap` if it's on screen.
    fn put_screen_tile(&mut self, emu: &mut Emulator, sx: i32, sy: i32, tile: u8, attr: u8) {
        let (tx, ty) = (self.anchor_tile.0 + sx, self.anchor_tile.1 + sy);
        let i = (ty.rem_euclid(32) * 32 + tx.rem_euclid(32)) as usize;
        self.vram_queue.push((0x1800 + i, tile));
        self.vram_queue.push((0x3800 + i, attr));
        if (0..20).contains(&sx) && (0..18).contains(&sy) {
            let k = (sy * 20 + sx) as u16;
            self.wram(emu, syms::W_TILEMAP.offset(k), tile);
            self.wram(emu, syms::W_ATTRMAP.offset(k), attr);
        }
    }

    /// Redraw the map around the anchor: the BG map with two tiles to spare
    /// on every side (what a step scrolls into view) and `wTilemap`.
    fn redraw_map(&mut self, emu: &mut Emulator) {
        for sy in -2..20 {
            for sx in -2..22 {
                let (raw, a) = self.map_tile(self.anchor_tile.0 + sx, self.anchor_tile.1 + sy);
                self.put_screen_tile(emu, sx, sy, raw & 0x7F, a);
            }
        }
    }

    fn draw_text_box(&mut self, emu: &mut Emulator, tb: &TextBox) {
        let pal = 0x07;
        for sy in 12..18 {
            for sx in 0..20 {
                let t = match (sx, sy) {
                    (0, 12) => 0x79,
                    (19, 12) => 0x7B,
                    (0, 17) => 0x7D,
                    (19, 17) => 0x7E,
                    (_, 12) | (_, 17) => 0x7A,
                    (0, _) | (19, _) => 0x7C,
                    _ => 0x7F,
                };
                self.put_screen_tile(emu, sx, sy, t, pal);
            }
        }
        let page = &tb.pages[tb.page];
        let mut left = tb.shown;
        for (row, line) in [(14, &page[0]), (16, &page[1])] {
            for (i, c) in line.chars().enumerate() {
                if left == 0 {
                    break;
                }
                left -= 1;
                self.put_screen_tile(emu, 1 + i as i32, row, char_tile(c), pal);
            }
        }
        if tb.typed() && (self.frame / 16) % 2 == 0 {
            self.put_screen_tile(emu, 18, 16, char_tile('v'), pal);
        }
    }

    fn menu_labels(&self) -> Vec<String> {
        let name = if self.name.is_empty() { "YOU" } else { &self.name };
        MENU_ITEMS
            .iter()
            .map(|(l, _)| l.unwrap_or(name).to_string())
            .collect()
    }

    fn draw_start_menu(&mut self, emu: &mut Emulator, cursor: usize) {
        let labels = self.menu_labels();
        let items: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
        self.draw_menu(emu, &items, cursor, 10, 0, 10);
    }

    /// The open menu, as Crystal's menu code leaves it in RAM: the start
    /// menu's header and items while it's open, the party screen's cursor.
    fn write_menu_ram(&mut self, emu: &mut Emulator) {
        let (bank, addr) = match self.mode {
            Mode::Menu(c) => {
                self.wram(emu, syms::W_MENU_ITEMS_LIST, MENU_ITEMS.len() as u8);
                for (k, (_, id)) in MENU_ITEMS.iter().enumerate() {
                    self.wram(emu, syms::W_MENU_ITEMS_LIST.offset(1 + k as u16), *id);
                }
                self.wram(emu, syms::W_MENU_CURSOR_Y, c as u8 + 1);
                (START_MENU_BANK, syms::START_MENU_ITEMS.addr)
            }
            Mode::Party { cursor, sub } => {
                self.wram(emu, syms::W_MENU_CURSOR_Y, sub.unwrap_or(cursor) as u8 + 1);
                self.wram(emu, syms::W_CUR_PARTY_MON, cursor as u8);
                (0, 0)
            }
            Mode::Dex { pos } => {
                // `DEXSTATE_UPDATE_MAIN_SCR`, the list scrolled to keep
                // the cursor in its 7 rows.
                let scroll = pos.saturating_sub(6);
                self.wram(emu, syms::W_JUMPTABLE_INDEX, 1);
                self.wram(emu, syms::W_DEX_LISTING_SCROLL_OFFSET, scroll as u8);
                self.wram(emu, syms::W_DEX_LISTING_CURSOR, (pos - scroll) as u8);
                for k in 0..16u16 {
                    let sp = DEX_ORDER.get(k as usize).copied().unwrap_or(0);
                    self.wram(emu, syms::W_POKEDEX_ORDER.offset(k), sp);
                }
                (0, 0)
            }
            _ => (0, 0),
        };
        self.wram(emu, syms::W_MENU_DATA_BANK, bank);
        self.wram16(emu, syms::W_MENU_DATA_POINTER_TABLE_ADDR, addr);
    }

    /// Crystal's party screen (`InitPartyMenuLayout`): ▶ at (0, 1 + 2i),
    /// nicknames from (3, 1 + 2i), HP from (13, 1 + 2i), level and HP bar
    /// on the row below, CANCEL after the last, a text box along the
    /// bottom. With the submenu open, the list's cursor goes (Crystal makes
    /// it hollow) and a box with STATS and CANCEL opens on the right.
    fn draw_party(&mut self, emu: &mut Emulator, cursor: usize, sub: Option<usize>) {
        let pal = 0x07;
        for sy in 0..18 {
            for sx in 0..20 {
                self.put_screen_tile(emu, sx, sy, 0x7F, pal);
            }
        }
        let text_at = |d: &mut Demo, emu: &mut Emulator, x: i32, y: i32, s: &str| {
            for (i, c) in s.chars().enumerate() {
                d.put_screen_tile(emu, x + i as i32, y, char_tile(c), pal);
            }
        };
        let n = mons::PARTY.len();
        for (i, m) in mons::PARTY.iter().enumerate() {
            let y = 1 + 2 * i as i32;
            text_at(self, emu, 3, y, m.nickname);
            if !m.egg {
                text_at(self, emu, 13, y, &format!("{:>3}/{:>3}", m.hp.0, m.hp.1));
                text_at(self, emu, 8, y + 1, &format!(":L{}", m.level));
                for x in 11..18 {
                    self.put_screen_tile(emu, x, y + 1, 0x50, 0x00);
                }
            }
        }
        text_at(self, emu, 3, 1 + 2 * n as i32, "CANCEL");
        if sub.is_none() {
            self.put_screen_tile(emu, 0, 1 + 2 * cursor as i32, char_tile('>'), pal);
        }
        let tb = TextBox {
            pages: vec![["CHOOSE A MON.".into(), String::new()]],
            page: 0,
            shown: 13,
            then: AfterText::Close,
        };
        self.draw_text_box(emu, &tb);
        if let Some(k) = sub {
            self.draw_menu(emu, &["STATS", "CANCEL"], k, 11, 7, 9);
        }
    }

    /// Crystal's Pokédex list (`Pokedex_DrawMainScreenBG` and
    /// `Pokedex_PrintListing`): the divider down column 8, SEEN and OWN on
    /// the left, seven entries on the right. (The real one has the
    /// selected Pokémon's picture top left; the demo leaves that to the
    /// cube.)
    fn draw_dex(&mut self, emu: &mut Emulator, pos: usize) {
        let pal = 0x07;
        for sy in 0..18 {
            for sx in 0..20 {
                let t = match (sx, sy) {
                    (8, 0) => 0x59,
                    (8, 8) => 0x53,
                    (8, 9) => 0x54,
                    (8, 16) => 0x5B,
                    (8, 1..=15) => 0x5A,
                    _ => 0x7F,
                };
                self.put_screen_tile(emu, sx, sy, t, pal);
            }
        }
        let text_at = |d: &mut Demo, emu: &mut Emulator, x: i32, y: i32, s: &str| {
            for (i, c) in s.chars().enumerate() {
                d.put_screen_tile(emu, x + i as i32, y, char_tile(c), pal);
            }
        };
        text_at(self, emu, 1, 11, "SEEN");
        text_at(self, emu, 5, 12, &format!("{:>3}", mons::SEEN.len()));
        text_at(self, emu, 1, 14, "OWN");
        text_at(self, emu, 5, 15, &format!("{:>3}", mons::CAUGHT.len()));
        let scroll = pos.saturating_sub(6);
        for k in 0..7 {
            let idx = scroll + k;
            let Some(&sp) = DEX_ORDER.get(idx) else { break };
            let y = 2 + 2 * k as i32;
            if idx == pos {
                self.put_screen_tile(emu, 10, y, char_tile('>'), pal);
            }
            let seen = mons::SEEN.contains(&sp);
            let name = if seen {
                mons::SPECIES[sp as usize - 1].name
            } else {
                "-----"
            };
            text_at(self, emu, 11, y, name);
        }
    }

    fn draw_menu(&mut self, emu: &mut Emulator, items: &[&str], cursor: usize, x0: i32, y0: i32, w: i32) {
        let pal = 0x07;
        let h = items.len() as i32 * 2 + 1;
        for sy in y0..=y0 + h {
            for sx in x0..x0 + w {
                let t = match (sx - x0, sy - y0) {
                    (0, 0) => 0x79,
                    (x, 0) if x == w - 1 => 0x7B,
                    (0, y) if y == h => 0x7D,
                    (x, y) if x == w - 1 && y == h => 0x7E,
                    (_, 0) => 0x7A,
                    (_, y) if y == h => 0x7A,
                    (0, _) => 0x7C,
                    (x, _) if x == w - 1 => 0x7C,
                    _ => 0x7F,
                };
                self.put_screen_tile(emu, sx, sy, t, pal);
            }
        }
        for (i, s) in items.iter().enumerate() {
            let y = y0 + 2 + 2 * i as i32;
            if i == cursor {
                self.put_screen_tile(emu, x0 + 1, y, char_tile('>'), pal);
            }
            for (k, c) in s.chars().enumerate() {
                self.put_screen_tile(emu, x0 + 2 + k as i32, y, char_tile(c), pal);
            }
        }
    }

    fn draw_battle(&mut self, emu: &mut Emulator, cursor: usize, text: Option<&TextBox>) {
        // Blank screen in the text palette.
        for sy in 0..18 {
            for sx in 0..20 {
                self.put_screen_tile(emu, sx, sy, 0x7F, 0x07);
            }
        }
        // Where Crystal puts things (`engine/battle/core.asm`): the
        // opponent's picture at (12, 0), yours (mirrored: from behind) at
        // (2, 6), the HUDs from (1, 0) and (9, 7), menus along the bottom.
        for ty in 0..6 {
            for tx in 0..6 {
                let t = 0x20 + (ty * 6 + tx) as u8;
                self.put_screen_tile(emu, 12 + tx, ty, t, 0x04);
                self.put_screen_tile(emu, 2 + (5 - tx), 6 + ty, t, 0x02 | 0x20);
            }
        }
        let text_at = |d: &mut Demo, emu: &mut Emulator, x: i32, y: i32, s: &str| {
            for (i, c) in s.chars().enumerate() {
                d.put_screen_tile(emu, x + i as i32, y, char_tile(c), 0x07);
            }
        };
        text_at(self, emu, 1, 0, "BLOB");
        text_at(self, emu, 6, 1, ":L5");
        text_at(self, emu, 2, 2, "HP");
        for x in 4..10 {
            self.put_screen_tile(emu, x, 2, 0x50, 0x00);
        }
        text_at(self, emu, 10, 7, "CUBE");
        text_at(self, emu, 14, 8, ":L7");
        text_at(self, emu, 10, 9, "HP");
        for x in 12..18 {
            self.put_screen_tile(emu, x, 9, 0x50, 0x00);
        }
        text_at(self, emu, 11, 10, "23/ 23");
        match text {
            Some(tb) => self.draw_text_box(emu, tb),
            None => {
                let empty = TextBox {
                    pages: vec![[String::new(), String::new()]],
                    page: 0,
                    shown: 0,
                    then: AfterText::Close,
                };
                self.draw_text_box(emu, &empty);
                // The battle menu, Crystal-style: a box on the right with
                // two items a row.
                let pal = 0x07;
                for sy in 12..18 {
                    for sx in 8..20 {
                        let t = match (sx, sy) {
                            (8, 12) => 0x79,
                            (19, 12) => 0x7B,
                            (8, 17) => 0x7D,
                            (19, 17) => 0x7E,
                            (_, 12) | (_, 17) => 0x7A,
                            (8, _) | (19, _) => 0x7C,
                            _ => 0x7F,
                        };
                        self.put_screen_tile(emu, sx, sy, t, pal);
                    }
                }
                for (i, s) in BATTLE_ITEMS.iter().enumerate() {
                    let (x, y) = (10 + (i % 2) as i32 * 6, 14 + (i / 2) as i32 * 2);
                    if i == cursor {
                        self.put_screen_tile(emu, x - 1, y, char_tile('>'), pal);
                    }
                    text_at(self, emu, x, y, s);
                }
                text_at(self, emu, 1, 14, "WHAT");
                text_at(self, emu, 1, 16, "NOW?");
            }
        }
    }

    /// The professor's scene, Crystal-style: a blank screen with a picture
    /// in the middle (7×7 tiles at (6, 4) in Crystal; the blob is 6×6) and
    /// the speech in the text box.
    fn draw_intro(&mut self, emu: &mut Emulator) {
        self.set_battle_palettes(true);
        for sy in 0..18 {
            for sx in 0..20 {
                self.put_screen_tile(emu, sx, sy, 0x7F, 0x07);
            }
        }
        for ty in 0..6 {
            for tx in 0..6 {
                self.put_screen_tile(emu, 7 + tx, 4 + ty, 0x20 + (ty * 6 + tx) as u8, 0x04);
            }
        }
    }

    /// Crystal's naming screen (`NamingScreen_InitText` and friends): ■
    /// all round, the prompt at (5, 2), the name at (5, 6), the keys every
    /// other tile from (2, 8), UPPER/DEL/END on row 16. The cursor is a
    /// sprite in the real game; here only its struct is written.
    fn draw_naming(&mut self, emu: &mut Emulator, n: &Naming) {
        let pal = 0x07;
        for sy in 0..18 {
            for sx in 0..20 {
                let inside = (1..19).contains(&sx) && ((1..7).contains(&sy) || (8..17).contains(&sy));
                self.put_screen_tile(emu, sx, sy, if inside { 0x7F } else { 0x60 }, pal);
            }
        }
        let text_at = |d: &mut Demo, emu: &mut Emulator, x: i32, y: i32, s: &str| {
            for (i, c) in s.chars().enumerate() {
                d.put_screen_tile(emu, x + i as i32, y, char_tile(c), pal);
            }
        };
        text_at(self, emu, 5, 2, "YOUR NAME?");
        for i in 0..NAME_LEN {
            let c = n.name.chars().nth(i).unwrap_or('-');
            self.put_screen_tile(emu, 5 + i as i32, 6, char_tile(c), pal);
        }
        for (r, keys) in KEYBOARD.iter().enumerate() {
            for (k, c) in keys.chars().enumerate() {
                self.put_screen_tile(emu, 2 + 2 * k as i32, 8 + 2 * r as i32, char_tile(c), pal);
            }
        }
        text_at(self, emu, 2, 16, KEYBOARD_CMDS);
        // The RAM the cube reads (see `gbc_cube_core::screens::Naming`).
        let s = Sym::new(0, CURSOR_STRUCT);
        self.wram16(emu, syms::W_NAMING_SCREEN_CURSOR_OBJECT_POINTER, CURSOR_STRUCT);
        self.wram(emu, s.offset(12), n.col);
        self.wram(emu, s.offset(13), n.row);
        self.wram16(
            emu,
            syms::W_NAMING_SCREEN_STRING_ENTRY_COORD,
            syms::W_TILEMAP.addr + 6 * 20 + 5,
        );
        self.wram(emu, syms::W_NAMING_SCREEN_MAX_NAME_LENGTH, NAME_LEN as u8);
        self.wram(emu, syms::W_NAMING_SCREEN_CUR_NAME_LENGTH, n.name.len() as u8);
    }

    /// One frame of the naming screen (`NamingScreenJoypadLoop`).
    fn naming(&mut self, emu: &mut Emulator, mut n: Naming, pressed: Buttons) -> Mode {
        let cmd = n.row == 4;
        if pressed.contains(Buttons::UP) {
            n.row = (n.row + 4) % 5;
        }
        if pressed.contains(Buttons::DOWN) {
            n.row = (n.row + 1) % 5;
        }
        // On the bottom row the cursor jumps between the three commands.
        let step = if cmd { 3 } else { 1 };
        if pressed.contains(Buttons::RIGHT) {
            n.col = (n.col / step * step + step) % 9;
        }
        if pressed.contains(Buttons::LEFT) {
            n.col = (n.col / step * step + 9 - step) % 9;
        }
        if pressed.contains(Buttons::START) {
            // To END.
            n.row = 4;
            n.col = 6;
        }
        let mut done = false;
        if pressed.contains(Buttons::B) {
            n.name.pop();
        }
        if pressed.contains(Buttons::A) {
            if n.row == 4 {
                match n.col / 3 {
                    1 => {
                        n.name.pop();
                    }
                    2 => done = true,
                    _ => {}
                }
            } else if n.name.len() < NAME_LEN {
                let c = KEYBOARD[n.row as usize].as_bytes()[n.col as usize] as char;
                if c != ' ' {
                    n.name.push(c);
                }
            }
        }
        if done {
            self.name = if n.name.is_empty() { "CUBE".into() } else { n.name };
            self.redraw_map(emu);
            let tb = TextBox::new(&format!("NICE TO MEET YOU, {}!", self.name), AfterText::Close);
            self.draw_text_box(emu, &tb);
            return Mode::Text(tb);
        }
        self.draw_naming(emu, &n);
        Mode::Naming(n)
    }

    fn set_battle_palettes(&mut self, on: bool) {
        // In battle, the monsters' palettes get a white background.
        let white = art::bgr(248, 248, 248);
        for pal in [2u8, 4] {
            let c = if on {
                white
            } else {
                art::BG_PALETTES[pal as usize][0]
            };
            self.pal_queue.push((pal * 8, c as u8));
            self.pal_queue.push((pal * 8 + 1, (c >> 8) as u8));
        }
    }

    fn logic(&mut self, emu: &mut Emulator, buttons: Buttons) {
        self.frame += 1;
        let pressed = Buttons::from_bits(buttons.bits() & !self.prev.bits());
        self.prev = buttons;

        // Tile animation: water and flowers every half second.
        if self.frame % 32 == 0 {
            let phase = (self.frame / 32) % 2 == 1;
            for (id, a, b) in [
                (bg::WATER, &art::WATER_A, &art::WATER_B),
                (bg::FLOWER, &art::FLOWER_A, &art::FLOWER_B),
            ] {
                let o = bg_tile_vram(id, false);
                for (i, v) in encode(if phase { b } else { a }).into_iter().enumerate() {
                    self.vram_queue.push((o + i, v));
                }
            }
        }

        let mode = std::mem::replace(&mut self.mode, Mode::Walk);
        self.mode = match mode {
            Mode::Walk => self.walk(emu, buttons, pressed),
            Mode::Text(mut tb) => {
                if !tb.typed() {
                    if self.frame % 2 == 0 || buttons.contains(Buttons::A) {
                        tb.shown += 1;
                    }
                    self.draw_text_box(emu, &tb);
                    Mode::Text(tb)
                } else if pressed.contains(Buttons::A) || pressed.contains(Buttons::B) {
                    if tb.page + 1 < tb.pages.len() {
                        tb.page += 1;
                        tb.shown = 0;
                        self.draw_text_box(emu, &tb);
                        Mode::Text(tb)
                    } else if tb.then == AfterText::Naming {
                        self.set_battle_palettes(false);
                        let n = Naming::default();
                        self.draw_naming(emu, &n);
                        Mode::Naming(n)
                    } else {
                        self.redraw_map(emu);
                        Mode::Walk
                    }
                } else {
                    self.draw_text_box(emu, &tb);
                    Mode::Text(tb)
                }
            }
            Mode::Menu(mut c) => {
                if pressed.contains(Buttons::UP) {
                    c = (c + MENU_ITEMS.len() - 1) % MENU_ITEMS.len();
                }
                if pressed.contains(Buttons::DOWN) {
                    c = (c + 1) % MENU_ITEMS.len();
                }
                let item = MENU_ITEMS[c].1;
                if pressed.contains(Buttons::B)
                    || pressed.contains(Buttons::START)
                    || (pressed.contains(Buttons::A) && item == 6)
                {
                    self.redraw_map(emu);
                    Mode::Walk
                } else if pressed.contains(Buttons::A) && item == 1 {
                    self.draw_party(emu, 0, None);
                    Mode::Party { cursor: 0, sub: None }
                } else if pressed.contains(Buttons::A) && item == 0 {
                    self.draw_dex(emu, 0);
                    Mode::Dex { pos: 0 }
                } else if pressed.contains(Buttons::A) && item == 8 {
                    self.draw_intro(emu);
                    let tb = TextBox::new(SPEECH, AfterText::Naming);
                    self.draw_text_box(emu, &tb);
                    Mode::Text(tb)
                } else if pressed.contains(Buttons::A) {
                    self.redraw_map(emu);
                    let tb = TextBox::new(
                        &format!("{} ISN'T PART OF THIS DEMO.", self.menu_labels()[c]),
                        AfterText::Close,
                    );
                    self.draw_text_box(emu, &tb);
                    Mode::Text(tb)
                } else {
                    self.draw_start_menu(emu, c);
                    Mode::Menu(c)
                }
            }
            Mode::Party { mut cursor, mut sub } => {
                let n = mons::PARTY.len();
                let in_sub = sub.is_some();
                match sub {
                    None => {
                        if pressed.contains(Buttons::UP) && cursor > 0 {
                            cursor -= 1;
                        }
                        if pressed.contains(Buttons::DOWN) && cursor < n {
                            cursor += 1;
                        }
                        if pressed.contains(Buttons::A) && cursor < n {
                            sub = Some(0);
                        }
                    }
                    Some(k) => {
                        if pressed.contains(Buttons::UP) || pressed.contains(Buttons::DOWN) {
                            sub = Some(k ^ 1);
                        }
                        if pressed.contains(Buttons::B) || (pressed.contains(Buttons::A) && k == 1) {
                            sub = None;
                        }
                    }
                }
                let leave = !in_sub
                    && (pressed.contains(Buttons::B) || (pressed.contains(Buttons::A) && cursor == n));
                if leave {
                    // Back to the start menu, as Crystal does.
                    self.redraw_map(emu);
                    self.draw_start_menu(emu, 1);
                    Mode::Menu(1)
                } else {
                    self.draw_party(emu, cursor, sub);
                    Mode::Party { cursor, sub }
                }
            }
            Mode::Dex { mut pos } => {
                if pressed.contains(Buttons::UP) && pos > 0 {
                    pos -= 1;
                }
                if pressed.contains(Buttons::DOWN) && pos + 1 < DEX_ORDER.len() {
                    pos += 1;
                }
                if pressed.contains(Buttons::B) {
                    self.redraw_map(emu);
                    self.draw_start_menu(emu, 0);
                    Mode::Menu(0)
                } else {
                    self.draw_dex(emu, pos);
                    Mode::Dex { pos }
                }
            }
            Mode::Battle { mut cursor, text } => match text {
                Some(mut tb) => {
                    if !tb.typed() {
                        if self.frame % 2 == 0 || buttons.contains(Buttons::A) {
                            tb.shown += 1;
                        }
                        self.draw_battle(emu, cursor, Some(&tb));
                        Mode::Battle {
                            cursor,
                            text: Some(tb),
                        }
                    } else if pressed.contains(Buttons::A) || pressed.contains(Buttons::B) {
                        if tb.page + 1 < tb.pages.len() {
                            tb.page += 1;
                            tb.shown = 0;
                            self.draw_battle(emu, cursor, Some(&tb));
                            Mode::Battle {
                                cursor,
                                text: Some(tb),
                            }
                        } else if tb.then == AfterText::EndBattle {
                            self.end_battle(emu);
                            Mode::Walk
                        } else {
                            self.draw_battle(emu, cursor, None);
                            Mode::Battle { cursor, text: None }
                        }
                    } else {
                        self.draw_battle(emu, cursor, Some(&tb));
                        Mode::Battle {
                            cursor,
                            text: Some(tb),
                        }
                    }
                }
                None => {
                    if pressed.contains(Buttons::UP) || pressed.contains(Buttons::DOWN) {
                        cursor ^= 2;
                    }
                    if pressed.contains(Buttons::LEFT) || pressed.contains(Buttons::RIGHT) {
                        cursor ^= 1;
                    }
                    let msg = if pressed.contains(Buttons::A) {
                        Some(match cursor {
                            0 => "CUBE USED TUMBLE! THE WILD BLOB FAINTED!",
                            3 => "GOT AWAY SAFELY!",
                            _ => "NOT NOW! THE BLOB IS WATCHING.",
                        })
                    } else if pressed.contains(Buttons::B) {
                        Some("GOT AWAY SAFELY!")
                    } else {
                        None
                    };
                    let text = msg.map(|m| {
                        let ends = cursor == 0 || cursor == 3 || pressed.contains(Buttons::B);
                        TextBox::new(
                            m,
                            if ends {
                                AfterText::EndBattle
                            } else {
                                AfterText::BattleMenu
                            },
                        )
                    });
                    self.draw_battle(emu, cursor, text.as_ref());
                    Mode::Battle { cursor, text }
                }
            },
            Mode::Naming(n) => self.naming(emu, n, pressed),
        };

        self.write_objects(emu);
        self.write_menu_ram(emu);
        self.wram(emu, syms::W_BATTLE_MODE, u8::from(self.in_battle()));
        // `DelayFrame`: done, waiting for VBlank.
        self.wram(emu, syms::W_V_BLANK_OCCURRED, 1);
    }

    fn start_battle(&mut self, emu: &mut Emulator) -> Mode {
        self.set_battle_palettes(true);
        let tb = TextBox::new("A WILD BLOB APPEARED!", AfterText::BattleMenu);
        self.draw_battle(emu, 0, Some(&tb));
        Mode::Battle {
            cursor: 0,
            text: Some(tb),
        }
    }

    fn end_battle(&mut self, emu: &mut Emulator) {
        self.set_battle_palettes(false);
        self.redraw_map(emu);
    }

    fn walk(&mut self, emu: &mut Emulator, buttons: Buttons, pressed: Buttons) -> Mode {
        if self.player.moving.is_none() {
            if pressed.contains(Buttons::START) {
                self.draw_start_menu(emu, 0);
                return Mode::Menu(0);
            }
            if pressed.contains(Buttons::SELECT) {
                return self.start_battle(emu);
            }
            if pressed.contains(Buttons::A) {
                let (dx, dy) = self.player.facing.step();
                let (fx, fy) = (self.player.x + dx, self.player.y + dy);
                let say = if self.npcs.iter().any(|n| (n.0.x, n.0.y) == (fx, fy)) {
                    Some("TILT THE CUBE TO WALK. ROLL IT ONTO A NEW FACE AND THE TOWN ROLLS WITH IT!")
                } else if self.map_tile(fx * 2 + PAD as i32 * 4, fy * 2 + PAD as i32 * 4).0 == bg::SIGN {
                    Some("CUBE TOWN. THE WORLD WRAPS ROUND THE DIE. NORTH: ROUTE 1.")
                } else {
                    None
                };
                if let Some(s) = say {
                    let tb = TextBox::new(s, AfterText::Close);
                    self.draw_text_box(emu, &tb);
                    return Mode::Text(tb);
                }
            }
            if let Some(d) = Dir::from_buttons(buttons) {
                self.player.facing = d;
                let (dx, dy) = d.step();
                let (tx, ty) = (self.player.x + dx, self.player.y + dy);
                if self.walkable(tx, ty) && !self.occupied(tx, ty, None) {
                    self.player.moving = Some((d, 0));
                    // `UpdateOverworldMap`: the anchors jump a step ahead and
                    // the map is redrawn there; the scroll catches up.
                    self.move_anchor();
                    self.redraw_map(emu);
                }
            }
        } else {
            self.player.advance();
            if self.player.moving.is_none() {
                let (x, y) = (self.player.x, self.player.y);
                let in_grass =
                    self.map_tile(x * 2 + PAD as i32 * 4, y * 2 + PAD as i32 * 4).0 == bg::TALL_GRASS;
                if in_grass && self.rand() % 8 == 0 {
                    return self.start_battle(emu);
                }
            }
        }
        self.wander();
        Mode::Walk
    }

    fn wander(&mut self) {
        for i in 0..self.npcs.len() {
            let (mut n, home, mut wait) = self.npcs[i];
            if n.moving.is_some() {
                n.advance();
            } else if wait > 0 {
                wait -= 1;
            } else {
                wait = 40 + self.rand() % 80;
                let d = [Dir::Down, Dir::Up, Dir::Left, Dir::Right][(self.rand() % 4) as usize];
                n.facing = d;
                let (tx, ty) = (n.x + d.step().0, n.y + d.step().1);
                if (tx - home.0).abs() <= 3
                    && (ty - home.1).abs() <= 2
                    && self.walkable(tx, ty)
                    && !self.occupied(tx, ty, Some(i))
                {
                    n.moving = Some((d, 0));
                }
            }
            self.npcs[i] = (n, home, wait);
        }
    }

    /// The anchor follows the player's destination: the screen's top-left
    /// tile is 8 tiles up and left of the player's cell.
    fn move_anchor(&mut self) {
        let (tx, ty) = self.player.target().unwrap_or((self.player.x, self.player.y));
        self.anchor_tile = (tx * 2 - 8 + PAD as i32 * 4, ty * 2 - 8 + PAD as i32 * 4);
    }

    /// Anchors, scroll, object structs and the shadow OAM for the next
    /// frame.
    fn write_objects(&mut self, emu: &mut Emulator) {
        if self.frame == 1 {
            self.move_anchor();
            self.redraw_map(emu);
        }
        let (atx, aty) = self.anchor_tile;
        let (bx, by) = (atx.div_euclid(4), aty.div_euclid(4));
        let anchor = syms::W_OVERWORLD_MAP_BLOCKS.addr + (by as usize * STRIDE + bx as usize) as u16;
        self.wram16(emu, syms::W_OVERWORLD_MAP_ANCHOR, anchor);
        self.wram(emu, syms::W_PLAYER_METATILE_X, (atx.rem_euclid(4) / 2) as u8);
        self.wram(emu, syms::W_PLAYER_METATILE_Y, (aty.rem_euclid(4) / 2) as u8);
        let bg_col = atx.rem_euclid(32);
        let bg_row = aty.rem_euclid(32);
        self.wram16(emu, syms::W_BG_MAP_ANCHOR, 0x9800 + (bg_row * 32 + bg_col) as u16);
        // The camera trails the anchor by what's left of the step.
        let p = self.player.px();
        let cam = (p.0 - 64, p.1 - 64);
        let (ax, ay) = (atx * 8, aty * 8);
        let scx = (bg_col * 8 + cam.0 - ax).rem_euclid(256) as u8;
        let scy = (bg_row * 8 + cam.1 - ay).rem_euclid(256) as u8;
        let battle = self.full_screen();
        // A battle screen sits still at the anchor.
        self.wram(emu, syms::H_SCX, if battle { (bg_col * 8) as u8 } else { scx });
        self.wram(emu, syms::H_SCY, if battle { (bg_row * 8) as u8 } else { scy });
        self.wram(emu, syms::W_X_COORD, self.player.x as u8);
        self.wram(emu, syms::W_Y_COORD, self.player.y as u8);
        self.wram(emu, syms::W_PLAYER_BG_MAP_OFFSET_X, 0);
        self.wram(emu, syms::W_PLAYER_BG_MAP_OFFSET_Y, 0);

        // Object structs: the player, the two townsfolk, the rest empty.
        let people = [
            (self.player, 1u8, 0x00u8, 0u8),
            (self.npcs[0].0, 2, 0x18, 1),
            (self.npcs[1].0, 3, 0x80, 2),
        ];
        let mut oam = [0u8; 0xA0];
        let mut n = 0;
        for slot in 0..obj::COUNT {
            let s = syms::W_OBJECT_STRUCTS.offset(slot * obj::LENGTH);
            for f in 0..obj::LENGTH {
                self.wram(emu, s.offset(f), 0);
            }
            let Some(&(m, sprite, tile, pal)) = people.get(slot as usize) else {
                continue;
            };
            // Crystal only keeps object structs for objects near the screen
            // (`CheckObjectStillVisible`: 5 steps left and up of the player
            // to 6 right and 5 down).
            let (dx, dy) = (m.x - self.player.x, m.y - self.player.y);
            if !(-5..=6).contains(&dx) || !(-5..=5).contains(&dy) {
                continue;
            }
            let (wx, wy) = m.px();
            let (sx, sy) = (wx - cam.0, wy - cam.1);
            self.wram(emu, s.offset(obj::SPRITE), sprite);
            self.wram(emu, s.offset(obj::SPRITE_TILE), tile);
            self.wram(emu, s.offset(obj::PALETTE), pal);
            self.wram(emu, s.offset(obj::FACING), m.facing_value());
            self.wram(emu, s.offset(obj::MAP_X), (m.x + 4) as u8);
            self.wram(emu, s.offset(obj::MAP_Y), (m.y + 4) as u8);
            self.wram(emu, s.offset(obj::SPRITE_X), sx as u8);
            self.wram(emu, s.offset(obj::SPRITE_Y), sy as u8);
            // The shadow OAM, built here independently of the cube's
            // `objects()`: only what's on screen, 16×16 from four tiles,
            // drawn 4 px above the cell.
            if battle || !(-16..176).contains(&sx) || !(-16..160).contains(&sy) {
                continue;
            }
            let bank1 = tile & 0x80 == 0;
            let base = tile & 0x7F;
            let (frame, flip) = match (m.facing, m.facing_value() % 4) {
                (Dir::Down, 1) => (frames::DOWN_WALK, false),
                (Dir::Down, 3) => (frames::DOWN_WALK, true),
                (Dir::Down, _) => (frames::DOWN, false),
                (Dir::Up, 1) => (frames::UP_WALK, false),
                (Dir::Up, 3) => (frames::UP_WALK, true),
                (Dir::Up, _) => (frames::UP, false),
                (Dir::Left, v) => (
                    if v % 2 == 1 {
                        frames::LEFT_WALK
                    } else {
                        frames::LEFT
                    },
                    false,
                ),
                (Dir::Right, v) => (
                    if v % 2 == 1 {
                        frames::LEFT_WALK
                    } else {
                        frames::LEFT
                    },
                    true,
                ),
            };
            let attr = pal | if bank1 { 0x08 } else { 0 } | if flip { 0x20 } else { 0 };
            for k in 0..4u8 {
                let (cx, cy) = ((k % 2) as i32 * 8, (k / 2) as i32 * 8);
                let cx = if flip { 8 - cx } else { cx };
                let e = &mut oam[n * 4..n * 4 + 4];
                e[0] = (sy - 4 + cy + 16) as u8;
                e[1] = (sx + cx + 8) as u8;
                e[2] = base + frame + k;
                e[3] = attr;
                n += 1;
            }
        }
        let base = syms::W_SHADOW_OAM.addr as usize - 0xC000;
        emu.mem_mut().wram[base..base + 0xA0].copy_from_slice(&oam);
    }
}
