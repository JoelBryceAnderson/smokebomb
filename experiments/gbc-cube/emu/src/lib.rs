//! The Game Boy Color emulator behind a small, safe interface.
//!
//! The core is [walnut-cgb] (MIT), a single-header C emulator descended from
//! Peanut-GB, vendored under `vendor/walnut-cgb` and wrapped by
//! `csrc/shim.c`. This crate owns the ROM, the cartridge RAM and the `.sav`
//! file, and exposes what the cube needs:
//!
//! * [`Emulator::run_frame`] and the 160×144 RGB565 [`Emulator::frame`];
//! * WRAM / VRAM / OAM / I/O / palette RAM through [`Emulator::mem`], which
//!   implements the core's [`GbMem`] (so the cube renderer never sees this
//!   crate);
//! * the joypad ([`Emulator::set_buttons`]);
//! * [`Emulator::stats`]: ROM reads and bank switches, for the hardware
//!   feasibility numbers.
//!
//! [walnut-cgb]: https://github.com/Mr-PauI/walnut-cgb

use std::alloc::{self, Layout};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::mem::GbMem;
use gbc_cube_core::{LCD_H, LCD_W};

mod ffi;
pub mod rtc;

/// One CGB frame: 70 224 machine clocks at 4.194304 MHz (single speed; in
/// double speed the CPU runs twice as many in the same time).
pub const FRAME_HZ: f64 = 4_194_304.0 / 70_224.0;

#[derive(Debug)]
pub enum EmuError {
    /// `gb_init` refused the ROM (code from walnut-cgb's `gb_init_error_e`:
    /// 1 unsupported cartridge, 2 bad header checksum).
    Init(i32),
    /// The core hit an error mid-frame (invalid opcode, invalid read/write).
    Core {
        code: i32,
        addr: u16,
    },
    Io(std::io::Error),
}

impl fmt::Display for EmuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmuError::Init(1) => write!(f, "unsupported cartridge type"),
            EmuError::Init(2) => write!(f, "ROM header checksum is wrong (not a Game Boy ROM?)"),
            EmuError::Init(c) => write!(f, "emulator init failed ({c})"),
            EmuError::Core { code, addr } => write!(f, "emulator core error {code} at ${addr:04X}"),
            EmuError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EmuError {}

impl From<std::io::Error> for EmuError {
    fn from(e: std::io::Error) -> Self {
        EmuError::Io(e)
    }
}

/// CPU-side state worth showing in a debug view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuState {
    pub cgb_mode: bool,
    pub double_speed: bool,
    /// The CPU sits in `halt` (Crystal does between frames, in DelayFrame).
    pub halted: bool,
    pub wram_bank: u8,
    pub vram_bank: u8,
    pub rom_bank: u16,
    pub pc: u16,
    pub sp: u16,
}

/// What the ROM side did since the last [`Emulator::take_stats`].
#[derive(Clone, Debug, Default)]
pub struct RomStats {
    pub frames: u64,
    pub rom_reads: u64,
    /// Consecutive reads that landed in different 16 KiB banks.
    pub bank_changes: u64,
    pub cart_ram_writes: u64,
    /// Distinct 16 KiB banks read.
    pub banks_touched: u32,
    /// Reads per 16 KiB bank.
    pub bank_reads: Vec<u32>,
}

/// A running cartridge.
pub struct Emulator {
    ctx: NonNull<ffi::Gcb>,
    layout: Layout,
    // The C side keeps raw pointers into these two: never reallocate them.
    rom: Box<[u8]>,
    cart_ram: Box<[u8]>,
    save: Option<SaveFile>,
}

struct SaveFile {
    path: PathBuf,
    /// Cart RAM as last written to disk.
    written: Box<[u8]>,
    last_check: Instant,
}

// The context is only touched through `&mut self` / `&self`.
unsafe impl Send for Emulator {}

impl Emulator {
    /// Boot `rom` (post-boot-ROM state; no boot ROM is needed).
    pub fn new(rom: Vec<u8>) -> Result<Self, EmuError> {
        let rom = rom.into_boxed_slice();
        let size = unsafe { ffi::gcb_sizeof() };
        let layout = Layout::from_size_align(size, 64).expect("context layout");
        let raw = unsafe { alloc::alloc_zeroed(layout) } as *mut ffi::Gcb;
        let ctx = NonNull::new(raw).unwrap_or_else(|| alloc::handle_alloc_error(layout));
        let mut emu = Emulator {
            ctx,
            layout,
            rom,
            cart_ram: Box::new([]),
            save: None,
        };
        let r = unsafe { ffi::gcb_init(emu.ctx.as_ptr(), emu.rom.as_ptr(), emu.rom.len()) };
        if r != 0 {
            return Err(EmuError::Init(r));
        }
        let ram = unsafe { ffi::gcb_save_size(emu.ctx.as_ptr()) };
        emu.cart_ram = vec![0xFF; ram].into_boxed_slice();
        unsafe {
            ffi::gcb_attach_cart_ram(emu.ctx.as_ptr(), emu.cart_ram.as_mut_ptr(), emu.cart_ram.len());
            // gb_init reset the CPU before the cart RAM was attached; reset
            // again so nothing read the placeholder.
            ffi::gcb_reset(emu.ctx.as_ptr());
        }
        Ok(emu)
    }

    /// Load a ROM file and, if `save` is given, its battery save (created on
    /// the first write if it doesn't exist yet).
    pub fn open(rom_path: &Path, save: Option<PathBuf>) -> Result<Self, EmuError> {
        let mut emu = Self::new(fs::read(rom_path)?)?;
        if let Some(path) = save {
            emu.attach_save(path)?;
        }
        Ok(emu)
    }

    /// `game.gbc` → `game.sav`, next to the ROM.
    pub fn default_save_path(rom_path: &Path) -> PathBuf {
        rom_path.with_extension("sav")
    }

    /// Read the battery save at `path` if there is one, and write cart RAM
    /// back there from now on ([`Emulator::flush_save`]).
    pub fn attach_save(&mut self, path: PathBuf) -> Result<(), EmuError> {
        if self.cart_ram.is_empty() {
            return Ok(());
        }
        if let Ok(bytes) = fs::read(&path) {
            let n = bytes.len().min(self.cart_ram.len());
            self.cart_ram[..n].copy_from_slice(&bytes[..n]);
            if let Some(clock) = rtc::Footer::parse(&bytes[n..]) {
                let now = unix_now();
                let regs = clock.advanced_to(now);
                unsafe { ffi::gcb_set_rtc(self.ctx.as_ptr(), regs.as_ptr()) };
            }
        }
        self.save = Some(SaveFile {
            path,
            written: self.cart_ram.clone(),
            last_check: Instant::now(),
        });
        Ok(())
    }

    /// Write the save file if cart RAM changed since the last write. Cheap
    /// to call every frame: it only compares once a second, unless `now`.
    pub fn flush_save(&mut self, now: bool) -> Result<bool, EmuError> {
        let Some(save) = &mut self.save else {
            return Ok(false);
        };
        if !now && save.last_check.elapsed().as_secs_f32() < 1.0 {
            return Ok(false);
        }
        save.last_check = Instant::now();
        if save.written[..] == self.cart_ram[..] {
            return Ok(false);
        }
        let mut regs = [0u8; 5];
        unsafe { ffi::gcb_get_rtc(self.ctx.as_ptr(), regs.as_mut_ptr()) };
        let mut bytes = self.cart_ram.to_vec();
        bytes.extend_from_slice(
            &rtc::Footer {
                regs,
                unix_secs: unix_now(),
            }
            .to_bytes(),
        );
        // Write then rename, so a crash never leaves half a save.
        let tmp = save.path.with_extension("sav.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &save.path)?;
        save.written.copy_from_slice(&self.cart_ram);
        Ok(true)
    }

    pub fn save_path(&self) -> Option<&Path> {
        self.save.as_ref().map(|s| s.path.as_path())
    }

    /// Run until the PPU reaches line 144 (the start of vertical blank).
    /// Game code that runs at vblank (Crystal's VBlank handler) runs at the
    /// start of the next call.
    pub fn run_frame(&mut self) -> Result<(), EmuError> {
        let r = unsafe { ffi::gcb_run_frame(self.ctx.as_ptr()) };
        if r != 0 {
            let addr = unsafe { ffi::gcb_error_addr(self.ctx.as_ptr()) };
            return Err(EmuError::Core { code: r, addr });
        }
        Ok(())
    }

    /// The last frame, RGB565, row-major 160×144.
    pub fn frame(&self) -> &[u16; LCD_W * LCD_H] {
        unsafe { &*(ffi::gcb_framebuffer(self.ctx.as_ptr()) as *const [u16; LCD_W * LCD_H]) }
    }

    /// The last frame as CGB palette indexes (0x00–0x1F BG, 0x20–0x3F OBJ).
    pub fn frame_index(&self) -> &[u8; LCD_W * LCD_H] {
        unsafe { &*(ffi::gcb_framebuffer_index(self.ctx.as_ptr()) as *const [u8; LCD_W * LCD_H]) }
    }

    /// Turn the PPU's line drawing off (or back on). LCD timing, interrupts
    /// and STAT carry on; only the pixels aren't produced, so
    /// [`Emulator::frame`] goes stale. The cube's world renderer doesn't
    /// need them.
    pub fn set_lcd_drawing(&mut self, on: bool) {
        unsafe { ffi::gcb_set_lcd(self.ctx.as_ptr(), i32::from(on)) }
    }

    pub fn set_buttons(&mut self, b: Buttons) {
        unsafe { ffi::gcb_set_joypad(self.ctx.as_ptr(), b.bits()) }
    }

    pub fn cpu(&self) -> CpuState {
        let s = unsafe { ffi::gcb_state(self.ctx.as_ptr()) };
        CpuState {
            cgb_mode: s.cgb_mode != 0,
            double_speed: s.double_speed != 0,
            halted: s.halted != 0,
            wram_bank: s.wram_bank,
            vram_bank: s.vram_bank,
            rom_bank: s.rom_bank,
            pc: s.pc,
            sp: s.sp,
        }
    }

    pub fn take_stats(&mut self) -> RomStats {
        let mut s = ffi::GcbStats::default();
        unsafe { ffi::gcb_take_stats(self.ctx.as_ptr(), &mut s) };
        RomStats {
            frames: s.frames,
            rom_reads: s.rom_reads,
            bank_changes: s.bank_changes,
            cart_ram_writes: s.cart_ram_writes,
            banks_touched: s.banks_touched.iter().map(|b| b.count_ones()).sum(),
            bank_reads: s.bank_reads.to_vec(),
        }
    }

    /// Read through the CPU's memory map (banking, I/O side effects and all).
    pub fn read(&mut self, addr: u16) -> u8 {
        unsafe { ffi::gcb_read(self.ctx.as_ptr(), addr) }
    }

    /// Write through the CPU's memory map. Palette data writes (`$FF69`,
    /// `$FF6B`) go through here so the core's RGB565 palette follows.
    pub fn write(&mut self, addr: u16, val: u8) {
        unsafe { ffi::gcb_write(self.ctx.as_ptr(), addr, val) }
    }

    pub fn rom(&self) -> &[u8] {
        &self.rom
    }

    pub fn cart_ram(&self) -> &[u8] {
        &self.cart_ram
    }

    /// The cartridge title from the header (`$0134`–`$0142`).
    pub fn title(&self) -> String {
        rom_title(&self.rom)
    }

    /// Live, read-only view of the console's memory.
    pub fn mem(&self) -> Mem<'_> {
        let m = unsafe { ffi::gcb_mem(self.ctx.as_ptr()) };
        unsafe {
            Mem {
                wram: std::slice::from_raw_parts(m.wram, m.wram_len),
                vram: std::slice::from_raw_parts(m.vram, m.vram_len),
                oam: std::slice::from_raw_parts(m.oam, 0xA0),
                io: std::slice::from_raw_parts(m.hram_io, 0x100),
                bg_palette: std::slice::from_raw_parts(m.bg_palette, 64),
                obj_palette: std::slice::from_raw_parts(m.obj_palette, 64),
                rom: &self.rom,
            }
        }
    }

    /// Raw mutable access to the console's memory, bypassing the CPU. Only
    /// the demo cart uses this (it plays the part of the game's code).
    /// Palette RAM is left out: write it with [`Emulator::write`] so the
    /// core's converted copy stays in step.
    pub fn mem_mut(&mut self) -> MemMut<'_> {
        let m = unsafe { ffi::gcb_mem(self.ctx.as_ptr()) };
        unsafe {
            MemMut {
                wram: std::slice::from_raw_parts_mut(m.wram, m.wram_len),
                vram: std::slice::from_raw_parts_mut(m.vram, m.vram_len),
                oam: std::slice::from_raw_parts_mut(m.oam, 0xA0),
                io: std::slice::from_raw_parts_mut(m.hram_io, 0x100),
            }
        }
    }
}

impl Drop for Emulator {
    fn drop(&mut self) {
        let _ = self.flush_save(true);
        unsafe { alloc::dealloc(self.ctx.as_ptr() as *mut u8, self.layout) };
    }
}

/// A live view of the console's memory (see [`GbMem`]).
#[derive(Clone, Copy)]
pub struct Mem<'a> {
    wram: &'a [u8],
    vram: &'a [u8],
    oam: &'a [u8],
    io: &'a [u8],
    bg_palette: &'a [u8],
    obj_palette: &'a [u8],
    rom: &'a [u8],
}

impl GbMem for Mem<'_> {
    fn wram(&self) -> &[u8] {
        self.wram
    }
    fn vram(&self) -> &[u8] {
        self.vram
    }
    fn oam(&self) -> &[u8] {
        self.oam
    }
    fn io(&self) -> &[u8] {
        self.io
    }
    fn bg_palette(&self) -> &[u8] {
        self.bg_palette
    }
    fn obj_palette(&self) -> &[u8] {
        self.obj_palette
    }
    fn rom(&self, offset: u32) -> u8 {
        self.rom.get(offset as usize).copied().unwrap_or(0xFF)
    }
}

pub struct MemMut<'a> {
    pub wram: &'a mut [u8],
    pub vram: &'a mut [u8],
    pub oam: &'a mut [u8],
    pub io: &'a mut [u8],
}

pub fn rom_title(rom: &[u8]) -> String {
    let raw = rom.get(0x134..0x143).unwrap_or(&[]);
    raw.iter()
        .take_while(|&&b| b != 0)
        .filter(|b| b.is_ascii_graphic() || **b == b' ')
        .map(|&b| b as char)
        .collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
