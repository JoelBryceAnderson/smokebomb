//! The firmware core on a phone, behind a C ABI (`include/smokebomb_ffi.h`).
//!
//! The iOS AR viewer throws a virtual die with RealityKit physics, turns its
//! motion into IMU readings, and calls [`sb_die_tick`] once per firmware tick.
//! The core runs exactly as on the board: on the simulator HAL, with the clock
//! advanced by hand one tick at a time. Each face's panel comes back as RGBA
//! for a texture on that face.
//!
//! Nothing here knows about the chip; the boundary is the HAL, as for the
//! desktop simulator.

use std::mem::MaybeUninit;
use std::panic::{catch_unwind, AssertUnwindSafe};

use smokebomb_core::target::DisplayTarget;
use smokebomb_core::{Firmware, TICK_HZ};
use smokebomb_hal::{Face, Grey96, HapticEffect, ImuSample, Rgb64, Target, TargetId, FACE_COUNT};
use smokebomb_hal_simulator::{SimHandle, SimPlatform};

pub const SB_PANEL_GREY96: u8 = 0;
pub const SB_PANEL_RGB64: u8 = 1;

/// One IMU reading in the die's frame; see [`ImuSample`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SbImu {
    pub accel_mg: [i16; 3],
    pub gyro_mdps: [i32; 3],
}

impl From<SbImu> for ImuSample {
    fn from(s: SbImu) -> Self {
        ImuSample {
            accel_mg: s.accel_mg,
            gyro_mdps: s.gyro_mdps,
        }
    }
}

enum Core {
    Grey(Box<Firmware<SimPlatform<Grey96>>>),
    Rgb(Box<Firmware<SimPlatform<Rgb64>>>),
}

/// A running die.
pub struct SbDie {
    sim: SimHandle,
    core: Core,
    target: TargetId,
    side: u32,
    ticks: u64,
}

/// Boots the firmware in place on the heap (at ~150 KB it's too big for a
/// thread's stack), as the simulator server does.
fn boot<T: DisplayTarget>(sim: &SimHandle) -> Option<Box<Firmware<SimPlatform<T>>>> {
    let mut slot: Box<MaybeUninit<Firmware<SimPlatform<T>>>> = Box::new(MaybeUninit::uninit());
    Firmware::init(&mut slot, sim.peripherals_for::<T>()).ok()?;
    // SAFETY: `init` returned Ok, so it wrote every field.
    Some(unsafe { Box::from_raw(Box::into_raw(slot).cast()) })
}

impl SbDie {
    pub fn new(panel: u8) -> Option<Self> {
        let sim = SimHandle::new();
        {
            let mut s = sim.lock();
            s.manual_time_ms = Some(0);
            // Off the Nest, nothing magnetic nearby, on battery.
            s.nest_plugged = false;
            s.vbus = false;
            s.mag_field_mg = [0; 3];
        }
        let core = match panel {
            SB_PANEL_GREY96 => Core::Grey(boot::<Grey96>(&sim)?),
            SB_PANEL_RGB64 => Core::Rgb(boot::<Rgb64>(&sim)?),
            _ => return None,
        };
        let (target, side) = match core {
            Core::Grey(_) => (Grey96::ID, Grey96::WIDTH as u32),
            Core::Rgb(_) => (Rgb64::ID, Rgb64::WIDTH as u32),
        };
        Some(Self {
            sim,
            core,
            target,
            side,
            ticks: 0,
        })
    }

    /// One tick with this reading and touch mask; returns the frame counter.
    pub fn tick(&mut self, imu: ImuSample, touch_mask: u8) -> u64 {
        self.ticks += 1;
        {
            let mut s = self.sim.lock();
            s.manual_time_ms = Some(self.ticks * 1000 / TICK_HZ as u64);
            s.imu_resting = imu;
            s.touch_mask = touch_mask & 0x3f;
        }
        // A failed tick (a HAL error) is skipped, as the simulator does.
        let _ = match &mut self.core {
            Core::Grey(fw) => fw.tick(),
            Core::Rgb(fw) => fw.tick(),
        };
        self.sim.lock().frame_seq
    }

    pub fn side(&self) -> u32 {
        self.side
    }

    /// One face as RGBA8, top row first.
    pub fn face_rgba(&self, face: usize, out: &mut [u8]) -> bool {
        let pixels = (self.side * self.side) as usize;
        if face >= FACE_COUNT || out.len() < pixels * 4 {
            return false;
        }
        let s = self.sim.lock();
        if s.target != Some(self.target) {
            return false;
        }
        let frame = &s.faces[face];
        match self.target {
            TargetId::Grey96 => {
                // Two pixels a byte, high nibble first; 16 levels to 0–255.
                if frame.len() * 2 < pixels {
                    return false;
                }
                for (i, px) in out.chunks_exact_mut(4).take(pixels).enumerate() {
                    let b = frame[i / 2];
                    let level = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
                    let v = level * 17;
                    px.copy_from_slice(&[v, v, v, 255]);
                }
            }
            TargetId::Rgb64 => {
                // RGB565 high byte first, each channel's top bits repeated
                // into its bottom bits so full scale is 255.
                if frame.len() < pixels * 2 {
                    return false;
                }
                for (i, px) in out.chunks_exact_mut(4).take(pixels).enumerate() {
                    let v = u16::from_be_bytes([frame[2 * i], frame[2 * i + 1]]);
                    let r = ((v >> 11) & 0x1f) as u8;
                    let g = ((v >> 5) & 0x3f) as u8;
                    let b = (v & 0x1f) as u8;
                    px.copy_from_slice(&[(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2), 255]);
                }
            }
        }
        true
    }

    pub fn next_haptic(&mut self) -> Option<HapticEffect> {
        self.sim.lock().haptics.pop_front()
    }

    pub fn mode(&self) -> String {
        match &self.core {
            Core::Grey(fw) => format!("{:?}", fw.mode()),
            Core::Rgb(fw) => format!("{:?}", fw.mode()),
        }
    }

    pub fn set_local_time(&mut self, seconds: u32) {
        self.sim.lock().set_local_time(seconds % 86_400);
    }
}

/// The C number of a haptic effect: its position in [`HapticEffect`].
pub fn haptic_number(e: HapticEffect) -> i32 {
    use HapticEffect::*;
    match e {
        Tick => 0,
        Buzz => 1,
        LandingThud => 2,
        MaxCelebration => 3,
        Dud => 4,
        MenuOpen => 5,
        MenuTip => 6,
        MenuSave => 7,
        SeatThunk => 8,
        DockTick => 9,
        DoubleTap => 10,
        SoftBuzz => 11,
        Ready => 12,
    }
}

// --- C ABI ------------------------------------------------------------------
//
// Every entry point catches panics: unwinding into Swift is undefined
// behaviour. A panic reads as "nothing happened" (NULL, 0, false, -1).

fn guard<R>(fallback: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

#[no_mangle]
pub extern "C" fn sb_die_new(panel: u8) -> *mut SbDie {
    guard(std::ptr::null_mut(), || match SbDie::new(panel) {
        Some(die) => Box::into_raw(Box::new(die)),
        None => std::ptr::null_mut(),
    })
}

/// # Safety
/// `die` is NULL or came from [`sb_die_new`] and hasn't been freed.
#[no_mangle]
pub unsafe extern "C" fn sb_die_free(die: *mut SbDie) {
    if !die.is_null() {
        guard((), || drop(Box::from_raw(die)));
    }
}

/// # Safety
/// `die` is NULL or a live die.
#[no_mangle]
pub unsafe extern "C" fn sb_die_panel_side(die: *const SbDie) -> u32 {
    die.as_ref().map_or(0, SbDie::side)
}

/// # Safety
/// `die` is NULL or a live die, not used from another thread at the same time.
#[no_mangle]
pub unsafe extern "C" fn sb_die_tick(die: *mut SbDie, imu: SbImu, touch_mask: u8) -> u64 {
    match die.as_mut() {
        Some(d) => guard(0, || d.tick(imu.into(), touch_mask)),
        None => 0,
    }
}

/// # Safety
/// `die` is NULL or a live die; `out` points to `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn sb_die_face_rgba(die: *const SbDie, face: u8, out: *mut u8, len: usize) -> bool {
    let (Some(d), false) = (die.as_ref(), out.is_null()) else {
        return false;
    };
    let out = std::slice::from_raw_parts_mut(out, len);
    guard(false, || d.face_rgba(face as usize, out))
}

/// # Safety
/// `die` is NULL or a live die.
#[no_mangle]
pub unsafe extern "C" fn sb_die_next_haptic(die: *mut SbDie) -> i32 {
    match die.as_mut() {
        Some(d) => guard(-1, || d.next_haptic().map_or(-1, haptic_number)),
        None => -1,
    }
}

/// # Safety
/// `die` is NULL or a live die; `out` is NULL or points to `len` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn sb_die_mode(die: *const SbDie, out: *mut u8, len: usize) -> usize {
    let Some(d) = die.as_ref() else { return 0 };
    let text = guard(String::new(), || d.mode());
    if !out.is_null() && len > 0 {
        let n = text.len().min(len - 1);
        std::ptr::copy_nonoverlapping(text.as_ptr(), out, n);
        *out.add(n) = 0;
    }
    text.len()
}

/// # Safety
/// `die` is NULL or a live die.
#[no_mangle]
pub unsafe extern "C" fn sb_die_set_local_time(die: *mut SbDie, seconds: u32) {
    if let Some(d) = die.as_mut() {
        guard((), || d.set_local_time(seconds));
    }
}

/// The face order the C ABI uses, for callers in Rust.
pub const FACES: [Face; FACE_COUNT] = Face::ALL;
