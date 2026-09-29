//! Production-board HAL: nRF54L15 (Cortex-M33 @ 128 MHz) running Zephyr.
//!
//! **Stub.** Every driver returns [`HalError::NotImplemented`]. The plan is
//! to call Zephyr device drivers through a thin C shim (`zephyr/src/shim.c`)
//! exposed via `extern "C"`, keeping Rust the owner of all application logic.
//!
//! | Peripheral       | Part         | Bus                  |
//! |------------------|--------------|----------------------|
//! | 6x OLED 96x96    | SSD1317      | SPIM (shared DC/CLK, 6 CS) |
//! | IMU              | LSM6DSx      | TWIM                 |
//! | Magnetometer     | part TBD     | TWIM                 |
//! | Touch            | nRF54 COMP / ext. cap controller | GPIO |
//! | Secure element   | ATECC608B    | TWIM                 |
//! | PMIC / charger   | nPM1300      | TWIM                 |
//! | Haptics          | DRV2605L     | TWIM                 |
//! | Asset flash      | 512 Mbit QSPI NOR | QSPI (EXMIF)    |
//! | NFC              | NFCT (on-chip) | —                  |

#![no_std]

use smokebomb_hal::*;

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
        display: Ssd1317Array,
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

const TODO: HalError = HalError::NotImplemented;

pub struct Ssd1317Array;
impl Display for Ssd1317Array {
    fn write_frame(&mut self, _face: Face, _frame: &FrameBytes) -> HalResult<()> {
        Err(TODO) // DMA frame into the back buffer for this CS line
    }
    fn flush(&mut self) -> HalResult<()> {
        Err(TODO)
    }
    fn set_brightness(&mut self, _face: Face, _level: u8) -> HalResult<()> {
        Err(TODO) // SSD1317 0x81 contrast command
    }
    fn set_enabled(&mut self, _enabled: bool) -> HalResult<()> {
        Err(TODO) // 0xAE / 0xAF display off/on
    }
}

pub struct Lsm6dsx;
impl Imu for Lsm6dsx {
    fn read(&mut self) -> HalResult<Option<ImuSample>> {
        Err(TODO) // Zephyr sensor API, FIFO + free-fall/wake-up interrupts
    }
}

pub struct NestMag;
impl Magnetometer for NestMag {
    fn read(&mut self) -> HalResult<[i32; 3]> {
        Err(TODO) // one-shot measurement over I2C, with the panel supply paused
    }
    fn hard_iron(&self) -> [i32; 3] {
        [0; 3] // factory calibration, from the die's OTP page
    }
}

pub struct CapTouch;
impl Touch for CapTouch {
    fn read(&mut self) -> HalResult<u8> {
        Err(TODO)
    }
}

pub struct ZephyrBle;
impl Ble for ZephyrBle {
    fn is_connected(&self) -> bool {
        false
    }
    fn send(&mut self, _payload: &[u8]) -> HalResult<()> {
        Err(TODO) // bt_gatt_notify on the Smokebomb characteristic
    }
    fn receive(&mut self, _buf: &mut [u8]) -> HalResult<Option<usize>> {
        Err(TODO)
    }
    fn set_advertising(&mut self, _enabled: bool) -> HalResult<()> {
        Err(TODO)
    }
}

pub struct Atecc608;
impl SecureElement for Atecc608 {
    fn serial(&mut self) -> HalResult<[u8; 9]> {
        Err(TODO) // config zone bytes 0-3, 8-12
    }
    fn public_key(&mut self) -> HalResult<[u8; 64]> {
        Err(TODO) // GenKey (public) on slot 0
    }
    fn sign_digest(&mut self, _digest: &[u8; 32]) -> HalResult<[u8; 64]> {
        Err(TODO) // Nonce (passthrough) + Sign (external)
    }
    fn next_counter(&mut self) -> HalResult<u32> {
        Err(TODO) // Counter (increment) on counter 0
    }
}

pub struct Cracen;
impl Rng for Cracen {
    fn fill_bytes(&mut self, _buf: &mut [u8]) -> HalResult<()> {
        Err(TODO) // sys_csrand_get, reseeded from ATECC608 Random
    }
}

pub struct Drv2605l;
impl Haptics for Drv2605l {
    fn play(&mut self, _effect: HapticEffect) -> HalResult<()> {
        Err(TODO)
    }
}

pub struct Npm1300;
impl Power for Npm1300 {
    fn battery(&mut self) -> HalResult<BatteryStatus> {
        Err(TODO)
    }
    fn set_mode(&mut self, _mode: PowerMode) -> HalResult<()> {
        Err(TODO)
    }
}

pub struct QspiFlash;
impl AssetStore for QspiFlash {
    fn capacity(&self) -> u32 {
        64 * 1024 * 1024
    }
    fn read(&mut self, _offset: u32, _buf: &mut [u8]) -> HalResult<()> {
        Err(TODO) // flash_read on the QSPI NOR device
    }
}

pub struct Nfct;
impl Nfc for Nfct {
    fn set_payload(&mut self, _ndef: &[u8]) -> HalResult<()> {
        Err(TODO)
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
