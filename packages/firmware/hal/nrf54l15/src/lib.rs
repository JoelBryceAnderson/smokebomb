//! Production-board HAL: nRF54L15 (Cortex-M33 @ 128 MHz) running Zephyr.
//!
//! **Stub.** Every driver returns [`HalError::NotImplemented`]: it asks the
//! Zephyr app's C shim (`zephyr/src/shim.c`), whose `sb_hal_todo` always
//! says no. The plan is to call Zephyr device drivers through that shim,
//! keeping Rust the owner of all application logic; each driver replaces its
//! `todo` as it lands.
//!
//! | Peripheral       | Part         | Bus                  |
//! |------------------|--------------|----------------------|
//! | 6x OLED 96x96    | SSD1317      | SPIM (shared DC/CLK, 6 CS) |
//! | or 6x RGB 64x64 (`target-30-rgb64`) | SSD1357 | SPIM ≤ 10 MHz, 6 CS ([`ssd1357`]) |
//! | IMU              | LSM6DSx      | TWIM                 |
//! | Magnetometer     | part TBD     | TWIM                 |
//! | Touch            | nRF54 COMP / ext. cap controller | GPIO |
//! | Secure element   | ATECC608B    | TWIM                 |
//! | PMIC / charger   | nPM1300      | TWIM                 |
//! | Haptics          | DRV2605L     | TWIM                 |
//! | Asset flash      | 512 Mbit QSPI NOR | QSPI (EXMIF)    |
//! | NFC              | NFCT (on-chip) | —                  |

#![no_std]

pub mod ssd1357;

use smokebomb_hal::*;

pub struct Nrf54l15;

/// The panels this image drives, chosen by the `target-30-rgb64` feature:
/// the 34 mm die's SSD1317 96x96 grey (default) or the 30 mm die's SSD1357
/// 64x64 RGB.
#[cfg(not(feature = "target-30-rgb64"))]
pub type Panels = Ssd1317Array;
#[cfg(feature = "target-30-rgb64")]
pub type Panels = ssd1357::Ssd1357Array;

#[cfg(not(feature = "target-30-rgb64"))]
fn panels() -> Panels {
    Ssd1317Array
}

#[cfg(feature = "target-30-rgb64")]
fn panels() -> Panels {
    ssd1357::Ssd1357Array(ssd1357::Ssd1357::new(ssd1357::ZephyrPanels))
}

impl Platform for Nrf54l15 {
    type Display = Panels;
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
        display: panels(),
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

pub struct Ssd1317Array;
impl Display for Ssd1317Array {
    type Target = Grey96;

    fn write_frame(&mut self, _face: Face, _frame: &FrameBytes) -> HalResult<()> {
        todo(|| ()) // DMA frame into the back buffer for this CS line
    }
    fn flush(&mut self) -> HalResult<()> {
        todo(|| ())
    }
    fn set_brightness(&mut self, _face: Face, _level: u8) -> HalResult<()> {
        todo(|| ()) // SSD1317 0x81 contrast command
    }
    fn set_enabled(&mut self, _enabled: bool) -> HalResult<()> {
        todo(|| ()) // 0xAE / 0xAF display off/on
    }
}

pub struct Lsm6dsx;
impl Imu for Lsm6dsx {
    fn read(&mut self) -> HalResult<Option<ImuSample>> {
        todo(|| None) // Zephyr sensor API, FIFO + free-fall/wake-up interrupts
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

pub struct CapTouch;
impl Touch for CapTouch {
    fn read(&mut self) -> HalResult<u8> {
        todo(|| 0)
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
