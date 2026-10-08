//! Read-only access to the console's memory, whatever holds it.
//!
//! The simulator implements [`GbMem`] over the emulator's live arrays; on the
//! die it would sit over the emulator's RAM with ROM reads going through the
//! external-flash bank cache. Nothing here copies memory.

/// A symbol from the disassembly: bank and address. WRAM symbols at
/// `$D000`–`$DFFF` name their WRAMX bank (1–7); ROM symbols at
/// `$4000`–`$7FFF` their ROMX bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sym {
    pub bank: u8,
    pub addr: u16,
}

impl Sym {
    pub const fn new(bank: u8, addr: u16) -> Sym {
        Sym { bank, addr }
    }

    pub const fn offset(self, by: u16) -> Sym {
        Sym {
            bank: self.bank,
            addr: self.addr.wrapping_add(by),
        }
    }

    /// The ROM file offset of a ROM symbol.
    pub const fn rom_offset(self) -> u32 {
        if self.addr < 0x4000 {
            self.addr as u32
        } else {
            self.bank as u32 * 0x4000 + (self.addr as u32 - 0x4000)
        }
    }
}

pub trait GbMem {
    /// 32 KiB: bank 0 (`$C000`) then WRAMX banks 1–7, 4 KiB each.
    fn wram(&self) -> &[u8];
    /// 16 KiB: VRAM bank 0 (`$8000`–`$9FFF`) then bank 1.
    fn vram(&self) -> &[u8];
    /// 160 bytes of OAM.
    fn oam(&self) -> &[u8];
    /// `$FF00`–`$FFFF`: I/O registers, HRAM, IE.
    fn io(&self) -> &[u8];
    /// CGB background palette RAM, 8 palettes × 4 colours, BGR555 LE.
    fn bg_palette(&self) -> &[u8];
    /// CGB object palette RAM.
    fn obj_palette(&self) -> &[u8];
    /// A byte of the ROM file.
    fn rom(&self, offset: u32) -> u8;

    /// A WRAM byte, `$C000`–`$DFFF` (the symbol's bank for `$D000`+), or an
    /// HRAM/I/O byte at `$FF00`+.
    fn byte(&self, s: Sym) -> u8 {
        let a = s.addr as usize;
        match s.addr {
            0xC000..=0xCFFF => self.wram()[a - 0xC000],
            0xD000..=0xDFFF => self.wram()[(s.bank.clamp(1, 7) as usize) * 0x1000 + a - 0xD000],
            0xFF00..=0xFFFF => self.io()[a - 0xFF00],
            0x0000..=0x7FFF => self.rom(s.rom_offset()),
            _ => 0xFF,
        }
    }

    fn word(&self, s: Sym) -> u16 {
        self.byte(s) as u16 | (self.byte(s.offset(1)) as u16) << 8
    }

    /// A byte at a 16-bit address in ROM `bank` (bank 0 for `$0000`–`$3FFF`).
    fn rom_banked(&self, bank: u8, addr: u16) -> u8 {
        self.rom(Sym::new(bank, addr).rom_offset())
    }

    /// An I/O register by its low byte (`0x40` = LCDC).
    fn reg(&self, r: u8) -> u8 {
        self.io()[r as usize]
    }
}

/// Hardware registers used here (`$FF00` + n).
pub mod reg {
    pub const LCDC: u8 = 0x40;
    pub const SCY: u8 = 0x42;
    pub const SCX: u8 = 0x43;
    pub const WY: u8 = 0x4A;
    pub const WX: u8 = 0x4B;
}
