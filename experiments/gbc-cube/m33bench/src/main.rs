//! How many Cortex-M33 instructions the emulator and the cube renderer take
//! a frame.
//!
//! Runs on QEMU's mps2-an505 (Cortex-M33) with `-icount shift=0`: every
//! executed instruction advances virtual time by 1 ns, so a timer measures
//! instructions. SysTick counts the virtual clock; a loop of known length
//! calibrates ticks per instruction. Results go out over semihosting.
//!
//! QEMU doesn't model the pipeline, caches or memory wait states, so these
//! are instruction counts, not cycles: see GBC_CUBE_HW_FEASIBILITY.md for
//! the conversion.

#![no_std]
#![no_main]

use core::fmt::Write;
use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::peripheral::syst::SystClkSource;
use cortex_m_rt::{entry, exception};
use gbc_cube_core::crystal::screen;
use gbc_cube_core::crystal::world::{self, BlockCache, Camera, MapInfo};
use gbc_cube_core::cube::{Config, Cube, FrameIn};
use gbc_cube_core::fallback::{self, Fallback};
use gbc_cube_core::geom::{Heading, Layout, Role};
use gbc_cube_core::mem::GbMem;
use gbc_cube_core::view::View;
use gbc_cube_core::FaceBuf;

mod data {
    include!(concat!(env!("OUT_DIR"), "/data.rs"));
}

// ---------------------------------------------------------------- output

fn semihost(op: u32, arg: *const u8) -> u32 {
    let r: u32;
    unsafe {
        core::arch::asm!("bkpt 0xAB", inout("r0") op => r, in("r1") arg);
    }
    r
}

struct Out;

impl Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut buf = [0u8; 128];
        for chunk in s.as_bytes().chunks(buf.len() - 1) {
            buf[..chunk.len()].copy_from_slice(chunk);
            buf[chunk.len()] = 0;
            semihost(0x04, buf.as_ptr()); // SYS_WRITE0
        }
        Ok(())
    }
}

fn exit() -> ! {
    // SYS_EXIT with ADP_Stopped_ApplicationExit.
    semihost(0x18, 0x20026 as *const u8);
    loop {
        cortex_m::asm::wfi();
    }
}

#[no_mangle]
pub extern "C" fn abort() -> ! {
    let _ = writeln!(Out, "abort (emulator error)");
    exit()
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let _ = writeln!(Out, "panic: {info}");
    exit()
}

// ---------------------------------------------------------------- timing

static WRAPS: AtomicU32 = AtomicU32::new(0);
const RELOAD: u32 = 0x00FF_FFFF;

#[exception]
fn SysTick() {
    WRAPS.fetch_add(1, Ordering::Relaxed);
}

/// Virtual clock ticks since start.
fn ticks() -> u64 {
    loop {
        let w = WRAPS.load(Ordering::Relaxed);
        let v = cortex_m::peripheral::SYST::get_current();
        if WRAPS.load(Ordering::Relaxed) == w {
            return w as u64 * (RELOAD as u64 + 1) + (RELOAD - v) as u64;
        }
    }
}

/// Instructions per tick, from a loop of known length.
fn calibrate() -> f64 {
    const N: u32 = 4_000_000;
    let t0 = ticks();
    unsafe {
        // Two instructions an iteration.
        core::arch::asm!("2:", "subs {n}, #1", "bne 2b", n = inout(reg) N => _);
    }
    let t1 = ticks();
    let t = t1 - t0;
    (2 * N) as f64 / t as f64
}

struct Meter {
    per_tick: f64,
}

impl Meter {
    /// Instructions taken by `f`.
    fn count(&self, f: impl FnOnce()) -> u64 {
        let t0 = ticks();
        f();
        ((ticks() - t0) as f64 * self.per_tick) as u64
    }
}

// ---------------------------------------------------------------- emulator

#[repr(C)]
struct Gcb {
    _p: [u8; 0],
}

#[repr(C)]
struct GcbState {
    cgb_mode: u8,
    double_speed: u8,
    halted: u8,
    wram_bank: u8,
    vram_bank: u8,
    rom_bank: u16,
    pc: u16,
    sp: u16,
}

extern "C" {
    fn gcb_sizeof() -> usize;
    fn gcb_init(c: *mut Gcb, rom: *const u8, rom_len: usize) -> i32;
    fn gcb_save_size(c: *mut Gcb) -> usize;
    fn gcb_attach_cart_ram(c: *mut Gcb, ram: *mut u8, len: usize);
    fn gcb_reset(c: *mut Gcb);
    fn gcb_run_frame(c: *mut Gcb) -> i32;
    fn gcb_state(c: *const Gcb) -> GcbState;
    fn gcb_set_joypad(c: *mut Gcb, pressed: u8);
    fn gcb_set_lcd(c: *mut Gcb, draw: i32);
}

/// Joypad for frame `i`: A every second through the intro and the title
/// (with a save, that's CONTINUE), then a walk in a square.
fn script(i: u32) -> u8 {
    const A: u8 = 0x01;
    if i < WARMUP {
        return if i % 60 < 4 { A } else { 0 };
    }
    [0x40, 0x10, 0x80, 0x20][((i / 64) % 4) as usize]
}

/// Frames run before measuring: `GBC_CUBE_M33_WARMUP` at build time, 1500 by
/// default (long enough for a game to get from its intro to the map).
const WARMUP: u32 = match option_env!("GBC_CUBE_M33_WARMUP") {
    Some(s) => parse(s),
    None => 1500,
};

const fn parse(s: &str) -> u32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut n = 0;
    while i < b.len() {
        n = n * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    n
}

#[repr(C, align(64))]
struct Ctx([u8; 128 * 1024]);
static mut CTX: Ctx = Ctx([0; 128 * 1024]);
static mut CART_RAM: [u8; 32 * 1024] = [0; 32 * 1024];

fn bench_emulator(m: &Meter) {
    let rom = data::CPU_ROM;
    if rom.is_empty() {
        let _ = writeln!(Out, "emulator: no ROM embedded (set GBC_CUBE_M33_ROM)");
        return;
    }
    unsafe {
        let size = gcb_sizeof();
        let _ = writeln!(Out, "emulator context: {size} bytes");
        assert!(size <= core::mem::size_of::<Ctx>());
        let c = addr_of_mut!(CTX) as *mut Gcb;
        let r = gcb_init(c, rom.as_ptr(), rom.len());
        assert!(r == 0, "gcb_init {}", r);
        let ram = gcb_save_size(c).min(32 * 1024);
        let cart = &mut *addr_of_mut!(CART_RAM);
        let sav = data::CPU_SAV;
        let n = sav.len().min(ram);
        cart[..n].copy_from_slice(&sav[..n]);
        gcb_attach_cart_ram(c, cart.as_mut_ptr(), ram);
        gcb_reset(c);
        let _ = writeln!(
            Out,
            "emulator: {} KiB ROM, {} B save loaded; {WARMUP} frames of warm-up",
            rom.len() / 1024,
            n
        );
        for i in 0..WARMUP {
            gcb_set_joypad(c, script(i));
            gcb_run_frame(c);
        }
        let frames = 600u64;
        // [single speed, double speed]: (frames, instructions, max)
        let mut by_speed = [(0u64, 0u64, 0u64); 2];
        for i in 0..frames as u32 {
            gcb_set_joypad(c, script(WARMUP + i));
            let n = m.count(|| {
                gcb_run_frame(c);
            });
            let k = usize::from(gcb_state(c).double_speed != 0);
            by_speed[k].0 += 1;
            by_speed[k].1 += n;
            by_speed[k].2 = by_speed[k].2.max(n);
        }
        // The same frames again without drawing lines: what the emulator
        // costs while the cube draws the world from RAM instead.
        let mut no_lcd = 0u64;
        gcb_set_lcd(c, 0);
        for i in 0..frames as u32 {
            gcb_set_joypad(c, script(WARMUP + frames as u32 + i));
            no_lcd += m.count(|| {
                gcb_run_frame(c);
            });
        }
        gcb_set_lcd(c, 1);
        let all = by_speed[0].1 + by_speed[1].1;
        let _ = writeln!(
            Out,
            "emulator: {} instructions/frame mean over {} frames",
            all / frames,
            frames
        );
        for (k, name) in ["single speed", "double speed"].iter().enumerate() {
            let (n, sum, max) = by_speed[k];
            if n > 0 {
                let _ = writeln!(Out, "  {name}: {} frames, {} mean, {} max", n, sum / n, max);
            }
        }
        let _ = writeln!(
            Out,
            "  next {frames} frames, line drawing off: {} mean",
            no_lcd / frames
        );
    }
}

// ---------------------------------------------------------------- renderer

struct Snap {
    wram: &'static [u8],
    vram: &'static [u8],
    oam: &'static [u8],
    io: &'static [u8],
    bgpal: &'static [u8],
    objpal: &'static [u8],
}

impl GbMem for Snap {
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
        self.bgpal
    }
    fn obj_palette(&self) -> &[u8] {
        self.objpal
    }
    fn rom(&self, offset: u32) -> u8 {
        data::SNAP_ROM.get(offset as usize).copied().unwrap_or(0xFF)
    }
}

static mut VIEW: View = View::new();
static mut CACHE: BlockCache = BlockCache::new();
static mut FACES: [FaceBuf; 6] = [[0; 4096]; 6];
static mut FALLBACK: Fallback = Fallback::new();
static mut CUBE: MaybeUninit<Cube> = MaybeUninit::uninit();

fn bench_renderer(m: &Meter) {
    if data::SNAP_ROM.is_empty() {
        let _ = writeln!(Out, "renderer: no snapshot embedded (set GBC_CUBE_M33_SNAPSHOT)");
        return;
    }
    let walk = Snap {
        wram: data::walk::WRAM,
        vram: data::walk::VRAM,
        oam: data::walk::OAM,
        io: data::walk::IO,
        bgpal: data::walk::BGPAL,
        objpal: data::walk::OBJPAL,
    };
    let text = Snap {
        wram: data::text::WRAM,
        vram: data::text::VRAM,
        oam: data::text::OAM,
        io: data::text::IO,
        bgpal: data::text::BGPAL,
        objpal: data::text::OBJPAL,
    };
    let layout = Layout::new(Heading::default());
    let skip_bottom = 1 << layout.face_with(Role::Bottom).index();
    let (view, cache, faces, fb) = unsafe {
        (
            &mut *addr_of_mut!(VIEW),
            &mut *addr_of_mut!(CACHE),
            &mut *addr_of_mut!(FACES),
            &mut *addr_of_mut!(FALLBACK),
        )
    };
    let map = MapInfo::read(&walk).expect("map in the walk snapshot");
    let cam = Camera::read(&walk, &map).expect("camera");
    let reps = 10u64;
    let avg = |f: &mut dyn FnMut()| -> u64 { (0..reps).map(|_| m.count(&mut *f)).sum::<u64>() / reps };

    // Warm the metatile cache once, as a steady walk would.
    world::draw(&walk, &map, &cam, cache, view);
    let draw = avg(&mut || world::draw(&walk, &map, &cam, cache, view));
    let light = avg(&mut || view.compute_light(16));
    let drape = avg(&mut || view.drape_onto(&layout, faces, skip_bottom));
    let classify = avg(&mut || {
        screen::classify(&walk, Some(&map), Some(&cam), cache);
    });
    let frame: &[u8; 160 * 144] = data::walk::FRAME.try_into().expect("frame size");
    let pal = gbc_cube_core::color::palette_from_ram(walk.bgpal, walk.objpal);
    let from_frame = avg(&mut || view.from_frame(frame, &pal, (72, 72)));
    let _ = writeln!(Out, "renderer (instructions per frame, mean of {reps}):");
    let _ = writeln!(Out, "  phase 3 world::draw (192x192 canvas + objects): {draw}");
    let _ = writeln!(Out, "  fog light grid:                                 {light}");
    let _ = writeln!(Out, "  drape onto 5 faces:                             {drape}");
    let _ = writeln!(
        Out,
        "  screen classify (wTilemap vs map):              {classify}"
    );
    let _ = writeln!(
        Out,
        "  phase 2 canvas from the frame:                  {from_frame}"
    );

    // Text box: the front panel (C).
    let tmap = MapInfo::read(&text).expect("map in the text snapshot");
    let tcam = Camera::read(&text, &tmap).expect("camera");
    let info = screen::classify(&text, Some(&tmap), Some(&tcam), cache);
    fb.observe(&text, Some(&info), data::text::FRAME.try_into().unwrap());
    let panel = avg(&mut || {
        fallback::front_panel(&text, Some(&info), &layout, faces, fb);
    });
    let _ = writeln!(Out, "  C front panel (text box re-flowed):             {panel}");

    // The whole per-frame path, as the firmware would call it.
    let cube = unsafe { (*addr_of_mut!(CUBE)).write(Cube::new(Config::default())) };
    let f = FrameIn {
        mem: &walk,
        frame,
        palette: &pal,
        crystal: true,
    };
    let whole = avg(&mut || {
        cube.render(&f, faces);
    });
    let _ = writeln!(Out, "  Cube::render, overworld, everything:            {whole}");
}

#[entry]
fn main() -> ! {
    let mut p = cortex_m::Peripherals::take().unwrap();
    p.SYST.set_clock_source(SystClkSource::Core);
    p.SYST.set_reload(RELOAD);
    p.SYST.clear_current();
    p.SYST.enable_interrupt();
    p.SYST.enable_counter();
    // A cleared counter sits at 0 until its first tick reloads it (with no
    // interrupt): start measuring after that.
    while cortex_m::peripheral::SYST::get_current() == 0 {}
    let per_tick = calibrate();
    let _ = writeln!(Out, "calibration: {per_tick:.1} instructions per SysTick tick");
    let m = Meter { per_tick };
    bench_emulator(&m);
    bench_renderer(&m);
    exit()
}
