//! The GBC cube behind a C ABI (`include/gbc_cube_ffi.h`), for the iOS
//! app's Simulator tab.
//!
//! The tab already runs the die firmware this way: it moves a virtual die
//! with RealityKit, turns the motion into IMU readings, and calls a tick
//! function 60 times a second, then puts six 64×64 frames on the die's
//! faces. This crate does the same with a Game Boy Color emulator and the
//! experiment's cube logic in place of the firmware, so the tab doesn't
//! change.
//!
//! It also re-exports `smokebomb-ffi` whole: the app can only link one Rust
//! static library, so with the experiment switched on this library replaces
//! `libsmokebomb_ffi.a` and carries both ABIs.

use std::cell::RefCell;
use std::ffi::{c_char, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use gbc_cube_core::buttons::Buttons;
use gbc_cube_core::color::palette_from_ram;
use gbc_cube_core::crystal;
use gbc_cube_core::cube::{Config, Cube, FrameIn, Report, Wrap};
use gbc_cube_core::fallback::UiStyle;
use gbc_cube_core::mem::GbMem;
use gbc_cube_core::{FaceBuf, FACE, FACE_PIXELS};
use gbc_cube_demo::Demo;
use gbc_cube_emu::Emulator;
use smokebomb_hal::ImuSample;

pub use smokebomb_ffi;

/// One IMU reading; the same layout as `SbImu`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct GcImu {
    pub accel_mg: [i16; 3],
    pub gyro_mdps: [i32; 3],
}

pub const GC_UI_FRONT: u8 = 0;
pub const GC_UI_PAN: u8 = 1;
pub const GC_UI_SPREAD: u8 = 2;

/// A cartridge running on the cube.
pub struct GcCube {
    emu: Emulator,
    demo: Option<Demo>,
    cube: Box<Cube>,
    crystal: bool,
    title: String,
    faces: Box<[FaceBuf; 6]>,
    report: Option<Report>,
    ticks: u64,
}

thread_local! {
    static LAST_ERROR: RefCell<String> = const { RefCell::new(String::new()) };
}

fn set_error(e: impl ToString) {
    LAST_ERROR.with(|l| *l.borrow_mut() = e.to_string());
}

/// Copies `s` into `out` NUL-terminated and truncated to fit; returns its
/// full length.
fn copy_text(s: &str, out: *mut c_char, len: usize) -> usize {
    if !out.is_null() && len > 0 {
        let n = s.len().min(len - 1);
        // SAFETY: the caller gives `len` writable bytes at `out`.
        unsafe {
            std::ptr::copy_nonoverlapping(s.as_ptr(), out as *mut u8, n);
            *out.add(n) = 0;
        }
    }
    s.len()
}

impl GcCube {
    fn open(rom: Option<PathBuf>) -> Result<GcCube, String> {
        let (mut emu, demo) = match &rom {
            Some(path) => {
                let save = Emulator::default_save_path(path);
                let emu = Emulator::open(path, Some(save)).map_err(|e| format!("{}: {e}", path.display()))?;
                (emu, None)
            }
            None => (
                Emulator::new(gbc_cube_demo::rom()).map_err(|e| e.to_string())?,
                Some(Demo::new()),
            ),
        };
        if !emu.cpu().cgb_mode {
            return Err(format!("{} isn't a Game Boy Color game", emu.title()));
        }
        let title = emu.title();
        let crystal = demo.is_some() || crystal::is_crystal(&title);
        let mut demo = demo;
        if let Some(d) = &mut demo {
            d.boot(&mut emu);
        }
        Ok(GcCube {
            emu,
            demo,
            cube: Box::new(Cube::new(Config::default())),
            crystal,
            title,
            faces: Box::new([[0; FACE_PIXELS]; 6]),
            report: None,
            ticks: 0,
        })
    }

    fn tick(&mut self, imu: GcImu, touch: u8, keys: u8) -> u64 {
        // The tab ticks at exactly 60 Hz; the cube's clock follows.
        let now_ms = (self.ticks * 1000 / 60) as u32;
        self.ticks += 1;
        let sample = ImuSample {
            accel_mg: imu.accel_mg,
            gyro_mdps: imu.gyro_mdps,
        };
        let buttons = self.cube.sense(now_ms, &sample, touch) | Buttons::from_bits(keys);
        self.emu.set_buttons(buttons);
        if let Err(e) = self.emu.run_frame() {
            set_error(e);
            return self.ticks;
        }
        let mem = self.emu.mem();
        let palette = palette_from_ram(mem.bg_palette(), mem.obj_palette());
        let frame = FrameIn {
            mem: &mem,
            frame: self.emu.frame_index(),
            palette: &palette,
            crystal: self.crystal,
        };
        let report = self.cube.render(&frame, &mut self.faces);
        self.report = Some(report);
        if let Some(d) = &mut self.demo {
            d.step(&mut self.emu, buttons);
        }
        let _ = self.emu.flush_save(false);
        self.ticks
    }

    fn status(&self) -> String {
        let Some(r) = &self.report else {
            return "starting".into();
        };
        let up = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"][r.heading.up_face().index()];
        let drawn = match r.drawn {
            gbc_cube_core::cube::Drawn::World => "world from RAM",
            gbc_cube_core::cube::Drawn::Frame => "frame folded",
            gbc_cube_core::cube::Drawn::Pan => "A: pan",
            gbc_cube_core::cube::Drawn::Spread => "B: spread",
            gbc_cube_core::cube::Drawn::Front => "C: front face",
            gbc_cube_core::cube::Drawn::Naming => "C: keyboard",
            gbc_cube_core::cube::Drawn::Still => "C: still",
        };
        format!("{:?}, {drawn}, up {up}", r.scene)
    }
}

/// Runs `f`, turning a panic into `fallback` rather than unwinding into C.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| {
        set_error("internal error");
        fallback
    })
}

/// # Safety
/// `rom_path` is NULL or a NUL-terminated UTF-8 path.
#[no_mangle]
pub unsafe extern "C" fn gc_cube_open(rom_path: *const c_char) -> *mut GcCube {
    guard(std::ptr::null_mut(), || {
        set_error("");
        let rom = if rom_path.is_null() {
            None
        } else {
            // SAFETY: the caller passes a NUL-terminated string.
            match unsafe { CStr::from_ptr(rom_path) }.to_str() {
                Ok(p) => Some(PathBuf::from(p)),
                Err(_) => {
                    set_error("the ROM path isn't UTF-8");
                    return std::ptr::null_mut();
                }
            }
        };
        match GcCube::open(rom) {
            Ok(c) => Box::into_raw(Box::new(c)),
            Err(e) => {
                set_error(e);
                std::ptr::null_mut()
            }
        }
    })
}

/// # Safety
/// `out` has `len` writable bytes (or is NULL).
#[no_mangle]
pub unsafe extern "C" fn gc_cube_last_error(out: *mut c_char, len: usize) -> usize {
    LAST_ERROR.with(|l| copy_text(&l.borrow(), out, len))
}

/// # Safety
/// `cube` came from [`gc_cube_open`] and isn't used afterwards (or is NULL).
#[no_mangle]
pub unsafe extern "C" fn gc_cube_free(cube: *mut GcCube) {
    if !cube.is_null() {
        // SAFETY: from gc_cube_open's Box::into_raw. Dropping the emulator
        // writes the save if it changed.
        drop(unsafe { Box::from_raw(cube) });
    }
}

#[no_mangle]
pub extern "C" fn gc_cube_panel_side(_cube: *const GcCube) -> u32 {
    FACE as u32
}

/// # Safety
/// `cube` is a live cube from [`gc_cube_open`].
#[no_mangle]
pub unsafe extern "C" fn gc_cube_tick(cube: *mut GcCube, imu: GcImu, touch_mask: u8, keys: u8) -> u64 {
    // SAFETY: the caller's contract.
    let cube = unsafe { &mut *cube };
    guard(cube.ticks, || cube.tick(imu, touch_mask, keys))
}

/// # Safety
/// `cube` is a live cube; `out` has `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn gc_cube_face_rgba(cube: *const GcCube, face: u8, out: *mut u8, len: usize) -> bool {
    // SAFETY: the caller's contract.
    let cube = unsafe { &*cube };
    if face as usize >= 6 || out.is_null() || len < FACE_PIXELS * 4 {
        return false;
    }
    // SAFETY: checked above that `len` covers a face.
    let out = unsafe { std::slice::from_raw_parts_mut(out, FACE_PIXELS * 4) };
    for (px, &c) in out.chunks_exact_mut(4).zip(cube.faces[face as usize].iter()) {
        let (r, g, b) = ((c >> 11) as u8, ((c >> 5) & 0x3F) as u8, (c & 0x1F) as u8);
        px.copy_from_slice(&[(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2), 255]);
    }
    true
}

/// # Safety
/// `cube` is a live cube; `out` has `len` writable bytes (or is NULL).
#[no_mangle]
pub unsafe extern "C" fn gc_cube_status(cube: *const GcCube, out: *mut c_char, len: usize) -> usize {
    // SAFETY: the caller's contract.
    copy_text(&unsafe { &*cube }.status(), out, len)
}

/// # Safety
/// `cube` is a live cube; `out` has `len` writable bytes (or is NULL).
#[no_mangle]
pub unsafe extern "C" fn gc_cube_title(cube: *const GcCube, out: *mut c_char, len: usize) -> usize {
    // SAFETY: the caller's contract.
    copy_text(&unsafe { &*cube }.title, out, len)
}

/// # Safety
/// `cube` is a live cube.
#[no_mangle]
pub unsafe extern "C" fn gc_cube_set_ui(cube: *mut GcCube, style: u8) {
    // SAFETY: the caller's contract.
    let cube = unsafe { &mut *cube };
    cube.cube.cfg.ui = match style {
        GC_UI_PAN => UiStyle::Pan,
        GC_UI_SPREAD => UiStyle::Spread,
        _ => UiStyle::Front,
    };
}

/// # Safety
/// `cube` is a live cube.
#[no_mangle]
pub unsafe extern "C" fn gc_cube_set_world(cube: *mut GcCube, from_ram: bool) {
    // SAFETY: the caller's contract.
    let cube = unsafe { &mut *cube };
    cube.cube.cfg.wrap = if from_ram { Wrap::World } else { Wrap::Frame };
}
