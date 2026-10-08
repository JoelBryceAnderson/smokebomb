//! Bindings to `csrc/shim.h`. Keep in step with it.

#[repr(C)]
pub struct Gcb {
    _private: [u8; 0],
}

#[repr(C)]
pub struct GcbMem {
    pub wram: *mut u8,
    pub wram_len: usize,
    pub vram: *mut u8,
    pub vram_len: usize,
    pub oam: *mut u8,
    pub hram_io: *mut u8,
    pub bg_palette: *mut u8,
    pub obj_palette: *mut u8,
    pub fix_palette: *mut u16,
}

#[repr(C)]
pub struct GcbState {
    pub cgb_mode: u8,
    pub double_speed: u8,
    pub halted: u8,
    pub wram_bank: u8,
    pub vram_bank: u8,
    pub rom_bank: u16,
    pub pc: u16,
    pub sp: u16,
}

#[repr(C)]
pub struct GcbStats {
    pub rom_reads: u64,
    pub bank_changes: u64,
    pub cart_ram_writes: u64,
    pub frames: u64,
    pub banks_touched: [u8; 64],
    pub bank_reads: [u32; 512],
}

impl Default for GcbStats {
    fn default() -> Self {
        GcbStats {
            rom_reads: 0,
            bank_changes: 0,
            cart_ram_writes: 0,
            frames: 0,
            banks_touched: [0; 64],
            bank_reads: [0; 512],
        }
    }
}

extern "C" {
    pub fn gcb_sizeof() -> usize;
    pub fn gcb_init(c: *mut Gcb, rom: *const u8, rom_len: usize) -> i32;
    pub fn gcb_save_size(c: *mut Gcb) -> usize;
    pub fn gcb_attach_cart_ram(c: *mut Gcb, ram: *mut u8, len: usize);
    pub fn gcb_reset(c: *mut Gcb);
    pub fn gcb_run_frame(c: *mut Gcb) -> i32;
    pub fn gcb_error_addr(c: *const Gcb) -> u16;
    pub fn gcb_framebuffer(c: *const Gcb) -> *const u16;
    pub fn gcb_framebuffer_index(c: *const Gcb) -> *const u8;
    pub fn gcb_mem(c: *mut Gcb) -> GcbMem;
    pub fn gcb_state(c: *const Gcb) -> GcbState;
    pub fn gcb_read(c: *mut Gcb, addr: u16) -> u8;
    pub fn gcb_write(c: *mut Gcb, addr: u16, val: u8);
    pub fn gcb_set_lcd(c: *mut Gcb, draw: i32);
    pub fn gcb_set_joypad(c: *mut Gcb, pressed: u8);
    pub fn gcb_get_rtc(c: *const Gcb, out: *mut u8);
    pub fn gcb_set_rtc(c: *mut Gcb, input: *const u8);
    pub fn gcb_take_stats(c: *mut Gcb, out: *mut GcbStats);
}
