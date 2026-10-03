//! Production-board HAL: nRF54L15 (Cortex-M33 @ 128 MHz) running Zephyr.
//!
//! The drivers call Zephyr through the app's C side (`zephyr/src/shim.c`
//! and `zephyr/src/board.c`), keeping Rust the owner of all application
//! logic. The panels, IMU and touch are written; every other driver is
//! still a stub that fails with [`HalError::NotImplemented`] (it asks the
//! shim's `sb_hal_todo`, which always says no) until it lands.
//!
//! | Peripheral       | Part         | Bus                  |
//! |------------------|--------------|----------------------|
//! | 6x OLED 96x96    | SSD1317 ([`ssd1317`]) | SPIM00 (shared SCK/MOSI/DC/RESET, 6 CS) |
//! | IMU              | LSM6DSOX (Zephyr `st,lsm6dso`) | TWIM |
//! | Magnetometer     | MMC5603 or LIS2MDL (Zephyr `memsic,mmc56x3` / `st,lis2mdl`) | TWIM |
//! | Touch            | CAP1188 (Zephyr `microchip,cap12xx`) | TWIM |
//! | Secure element   | ATECC608B    | TWIM                 |
//! | PMIC / charger   | nPM1300      | TWIM                 |
//! | Haptics          | DRV2605L (Zephyr `ti,drv2605`) | TWIM |
//! | Asset flash      | 512 Mbit QSPI NOR | QSPI (EXMIF)    |
//! | NFC              | NFCT (on-chip) | —                  |
//!
//! The bench's pins are in `zephyr/BENCH.md`.

#![no_std]

use smokebomb_hal::*;

pub mod ssd1317;

use ssd1317::{Dc, PanelBus, Ssd1317};

pub struct Nrf54l15;

impl Platform for Nrf54l15 {
    type Display = Ssd1317Array;
    type Imu = Lsm6dsx;
    type Magnetometer = NestMag;
    type Touch = CapTouch;
    type Ble = ZephyrBle;
    type SecureElement = Atecc608;
    type Rng = Cracen;
    type Haptics = Drv2605l;
    type Power = Npm1300;
    type Assets = QspiFlash;
    type Nfc = Nfct;
    type Clock = KernelClock;
}

/// Take ownership of the board peripherals. Call once, after Zephyr has
/// initialised its devices.
pub fn peripherals() -> Peripherals<Nrf54l15> {
    Peripherals {
        display: Ssd1317Array::new(),
        imu: Lsm6dsx,
        mag: NestMag,
        touch: CapTouch,
        ble: ZephyrBle,
        secure_element: Atecc608,
        rng: Cracen,
        haptics: Drv2605l,
        power: Npm1300,
        assets: QspiFlash,
        nfc: Nfct,
        clock: KernelClock,
    }
}

extern "C" {
    /// The shim's stand-in for a driver: always fails (`-ENOSYS`).
    fn sb_hal_todo() -> i32;
    fn sb_sleep_us(us: u32);
    fn sb_panel_setup() -> i32;
    fn sb_panel_write(face: u8, data: bool, buf: *const u8, len: usize) -> i32;
    fn sb_panel_reset(asserted: bool) -> i32;
    fn sb_imu_read(accel_mg: *mut i16, gyro_mdps: *mut i32) -> i32;
    fn sb_touch_read(mask: *mut u8) -> i32;
}

/// `-ENODEV`: the part isn't on the board, or its driver didn't come up.
const ENODEV: i32 = 19;

/// A Zephyr return code as a [`HalResult`].
fn check(ret: i32) -> HalResult<()> {
    match ret {
        0.. => Ok(()),
        r if r == -ENODEV => Err(HalError::NotReady),
        _ => Err(HalError::Bus),
    }
}

/// A driver that isn't written yet: fails with
/// [`HalError::NotImplemented`]. It asks the shim rather than failing here
/// so the compiler can't tell it always fails; otherwise it would drop the
/// firmware after the first call at boot as unreachable, and the board
/// image would link almost none of it. `stand_in` is never used.
fn todo<T>(stand_in: impl FnOnce() -> T) -> HalResult<T> {
    // SAFETY: the shim takes no arguments and touches nothing.
    match unsafe { sb_hal_todo() } {
        0 => Ok(stand_in()),
        _ => Err(HalError::NotImplemented),
    }
}

/// The panels' bus: SPIM00 and the D/C and reset lines, through the shim.
pub struct ZephyrPanels;

impl PanelBus for ZephyrPanels {
    fn write(&mut self, panel: u8, dc: Dc, bytes: &[u8]) -> HalResult<()> {
        // SAFETY: the shim reads `bytes.len()` bytes from `bytes`, and is done
        // with them when it returns.
        check(unsafe { sb_panel_write(panel, dc == Dc::Data, bytes.as_ptr(), bytes.len()) })
    }
    fn set_reset(&mut self, asserted: bool) -> HalResult<()> {
        // SAFETY: drives one GPIO.
        check(unsafe { sb_panel_reset(asserted) })
    }
    fn delay_ms(&mut self, ms: u32) {
        // SAFETY: sleeps the calling thread.
        unsafe { sb_sleep_us(ms * 1000) }
    }
}

/// The six faces. They are reset and set up the first time they're turned
/// on.
pub struct Ssd1317Array {
    panels: Ssd1317<ZephyrPanels>,
    ready: bool,
}

impl Ssd1317Array {
    pub const fn new() -> Self {
        Self {
            panels: Ssd1317::new(ZephyrPanels),
            ready: false,
        }
    }

    fn bring_up(&mut self) -> HalResult<()> {
        // SAFETY: configures the panels' GPIOs; touches nothing of Rust's.
        check(unsafe { sb_panel_setup() })?;
        self.panels.reset()?;
        for face in 0..FACE_COUNT as u8 {
            self.panels.init(face)?;
        }
        self.ready = true;
        Ok(())
    }
}

impl Default for Ssd1317Array {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for Ssd1317Array {
    /// Written straight to the panel: the faces have no back buffer, so
    /// [`Display::flush`] has nothing to do.
    fn write_frame(&mut self, face: Face, frame: &FrameBytes) -> HalResult<()> {
        self.panels.write_frame(face as u8, frame)
    }
    fn flush(&mut self) -> HalResult<()> {
        Ok(())
    }
    fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()> {
        self.panels.set_contrast(face as u8, level)
    }
    fn set_enabled(&mut self, enabled: bool) -> HalResult<()> {
        if enabled && !self.ready {
            self.bring_up()?;
        }
        for face in 0..FACE_COUNT as u8 {
            self.panels.set_on(face, enabled)?;
        }
        Ok(())
    }
}

/// The LSM6DSOX, through Zephyr's sensor API. Its axes are taken as the
/// die's body axes: on the board they must be laid out to match.
pub struct Lsm6dsx;
impl Imu for Lsm6dsx {
    fn read(&mut self) -> HalResult<Option<ImuSample>> {
        let mut s = ImuSample::default();
        // SAFETY: the shim writes three values to each array.
        check(unsafe { sb_imu_read(s.accel_mg.as_mut_ptr(), s.gyro_mdps.as_mut_ptr()) })?;
        Ok(Some(s))
    }
}

pub struct NestMag;
impl Magnetometer for NestMag {
    fn read(&mut self) -> HalResult<[i32; 3]> {
        todo(|| [0; 3]) // one-shot measurement over I2C, with the panel supply paused
    }
    fn hard_iron(&self) -> [i32; 3] {
        [0; 3] // factory calibration, from the die's OTP page
    }
}

/// The CAP1188, through Zephyr's CAP12xx input driver: pad N is face N.
pub struct CapTouch;
impl Touch for CapTouch {
    fn read(&mut self) -> HalResult<u8> {
        let mut mask = 0;
        // SAFETY: the shim writes one byte to `mask`.
        check(unsafe { sb_touch_read(&mut mask) })?;
        Ok(mask)
    }
}

pub struct ZephyrBle;
impl Ble for ZephyrBle {
    fn is_connected(&self) -> bool {
        false
    }
    fn send(&mut self, _payload: &[u8]) -> HalResult<()> {
        todo(|| ()) // bt_gatt_notify on the Sugarcube characteristic
    }
    fn receive(&mut self, _buf: &mut [u8]) -> HalResult<Option<usize>> {
        todo(|| None)
    }
    fn set_advertising(&mut self, _enabled: bool) -> HalResult<()> {
        todo(|| ())
    }
}

pub struct Atecc608;
impl SecureElement for Atecc608 {
    fn serial(&mut self) -> HalResult<[u8; 9]> {
        todo(|| [0; 9]) // config zone bytes 0-3, 8-12
    }
    fn public_key(&mut self) -> HalResult<[u8; 64]> {
        todo(|| [0; 64]) // GenKey (public) on slot 0
    }
    fn sign_digest(&mut self, _digest: &[u8; 32]) -> HalResult<[u8; 64]> {
        todo(|| [0; 64]) // Nonce (passthrough) + Sign (external)
    }
    fn next_counter(&mut self) -> HalResult<u32> {
        todo(|| 0) // Counter (increment) on counter 0
    }
}

pub struct Cracen;
impl Rng for Cracen {
    fn fill_bytes(&mut self, _buf: &mut [u8]) -> HalResult<()> {
        todo(|| ()) // sys_csrand_get, reseeded from ATECC608 Random
    }
}

pub struct Drv2605l;
impl Haptics for Drv2605l {
    fn play(&mut self, _effect: HapticEffect) -> HalResult<()> {
        todo(|| ())
    }
}

pub struct Npm1300;
impl Power for Npm1300 {
    fn battery(&mut self) -> HalResult<BatteryStatus> {
        todo(|| BatteryStatus {
            percent: 0,
            millivolts: 0,
            vbus: false,
            charge: ChargeState::Idle,
        })
    }
    fn set_mode(&mut self, _mode: PowerMode) -> HalResult<()> {
        todo(|| ())
    }
}

pub struct QspiFlash;
impl AssetStore for QspiFlash {
    fn capacity(&self) -> u32 {
        64 * 1024 * 1024
    }
    fn read(&mut self, _offset: u32, _buf: &mut [u8]) -> HalResult<()> {
        todo(|| ()) // flash_read on the QSPI NOR device
    }
}

pub struct Nfct;
impl Nfc for Nfct {
    fn set_payload(&mut self, _ndef: &[u8]) -> HalResult<()> {
        todo(|| ())
    }
}

pub struct KernelClock;
impl Clock for KernelClock {
    fn now_ms(&self) -> u64 {
        0 // k_uptime_get()
    }
    fn local_seconds(&self) -> u32 {
        0 // RTC, set by the phone over BLE
    }
}
