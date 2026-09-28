//! Hardware abstraction layer for the Smokebomb die.
//!
//! The firmware core only talks to hardware through these traits. Two
//! implementations exist:
//!
//! * `smokebomb-hal-simulator` — in-memory mocks driven by the web simulator
//! * `smokebomb-hal-nrf54l15` — the real board (nRF54L15 + Zephyr drivers)
//!
//! Every trait is object-safe-ish and allocation-free so the same core code
//! runs on a Cortex-M33 and on a Mac.

#![no_std]

pub use smokebomb_shared::assets::{FRAME_BYTES, PANEL_HEIGHT, PANEL_WIDTH};
pub use smokebomb_shared::types::{Face, FACE_COUNT};

/// One packed 4bpp 96x96 panel frame.
pub type FrameBytes = [u8; FRAME_BYTES];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HalError {
    /// SPI/I2C/QSPI transfer failed.
    Bus,
    Timeout,
    /// Peripheral not initialised or powered down.
    NotReady,
    /// Argument out of range for this device.
    InvalidArgument,
    /// Driver stub; real implementation pending.
    NotImplemented,
}

pub type HalResult<T> = Result<T, HalError>;

/// Six SSD1317 96x96 OLED panels.
pub trait Display {
    /// Push a full frame to one face. Implementations may double-buffer and
    /// flip on [`Display::flush`].
    fn write_frame(&mut self, face: Face, frame: &FrameBytes) -> HalResult<()>;
    /// Latch all pending frames to the panels at once so faces change together.
    fn flush(&mut self) -> HalResult<()>;
    /// Panel contrast, 0-255.
    fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()>;
    fn set_enabled(&mut self, enabled: bool) -> HalResult<()>;
}

/// Raw 6-axis sample in the die's body frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImuSample {
    /// Acceleration in milli-g. At rest the face pointing up reads ~+1000 on
    /// its axis.
    pub accel_mg: [i16; 3],
    /// Angular rate in milli-degrees per second.
    pub gyro_mdps: [i32; 3],
}

/// LSM6DSx accelerometer + gyroscope.
pub trait Imu {
    /// Latest sample, or `None` if no new data since the last read.
    fn read(&mut self) -> HalResult<Option<ImuSample>>;
}

/// Capacitive touch pads, one per face.
pub trait Touch {
    /// Bitmask of currently touched faces (bit n = `Face` index n).
    fn read(&mut self) -> HalResult<u8>;
}

/// BLE peripheral link to the phone app. Framing is handled above this trait.
pub trait Ble {
    fn is_connected(&self) -> bool;
    /// Queue a notification on the Smokebomb characteristic.
    fn send(&mut self, payload: &[u8]) -> HalResult<()>;
    /// Copy the next received write into `buf`, returning its length.
    fn receive(&mut self, buf: &mut [u8]) -> HalResult<Option<usize>>;
    fn set_advertising(&mut self, enabled: bool) -> HalResult<()>;
}

/// ATECC608 secure element: device identity and roll signing.
pub trait SecureElement {
    fn serial(&mut self) -> HalResult<[u8; 9]>;
    /// Uncompressed P-256 public key, X || Y.
    fn public_key(&mut self) -> HalResult<[u8; 64]>;
    /// ECDSA-P256 over a pre-computed SHA-256 digest, r || s.
    fn sign_digest(&mut self, digest: &[u8; 32]) -> HalResult<[u8; 64]>;
    /// Increment and return the hardware monotonic counter.
    fn next_counter(&mut self) -> HalResult<u32>;
}

/// True random number generator (nRF54L15 CRACEN, mixed with ATECC608 RNG).
pub trait Rng {
    fn fill_bytes(&mut self, buf: &mut [u8]) -> HalResult<()>;

    fn next_u32(&mut self) -> HalResult<u32> {
        let mut b = [0u8; 4];
        self.fill_bytes(&mut b)?;
        Ok(u32::from_le_bytes(b))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HapticEffect {
    Tick,
    Buzz,
    LandingThud,
    MaxCelebration,
    Dud,
}

/// DRV2605L LRA driver.
pub trait Haptics {
    fn play(&mut self, effect: HapticEffect) -> HalResult<()>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerMode {
    Normal,
    LowPower,
    UltraLow,
    Sleep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryStatus {
    pub percent: u8,
    pub millivolts: u16,
    pub charging: bool,
    /// Sitting on the charging nest.
    pub docked: bool,
}

/// nPM1300 PMIC: charger, fuel gauge and rail control.
pub trait Power {
    fn battery(&mut self) -> HalResult<BatteryStatus>;
    fn set_mode(&mut self, mode: PowerMode) -> HalResult<()>;
}

/// Read-only view of the 64 MB QSPI flash holding pre-rendered animations.
pub trait AssetStore {
    fn capacity(&self) -> u32;
    fn read(&mut self, offset: u32, buf: &mut [u8]) -> HalResult<()>;
}

/// NFC tag used to hand a verified-session join link to phones.
pub trait Nfc {
    fn set_payload(&mut self, ndef: &[u8]) -> HalResult<()>;
}

/// Monotonic milliseconds since boot.
pub trait Clock {
    fn now_ms(&self) -> u64;
}

/// A complete board: one concrete type per peripheral. The core is generic
/// over `P: Platform`, so each board is monomorphised with zero dynamic
/// dispatch.
pub trait Platform {
    type Display: Display;
    type Imu: Imu;
    type Touch: Touch;
    type Ble: Ble;
    type SecureElement: SecureElement;
    type Rng: Rng;
    type Haptics: Haptics;
    type Power: Power;
    type Assets: AssetStore;
    type Nfc: Nfc;
    type Clock: Clock;
}

/// Owned peripheral set handed to the firmware core at boot.
pub struct Peripherals<P: Platform> {
    pub display: P::Display,
    pub imu: P::Imu,
    pub touch: P::Touch,
    pub ble: P::Ble,
    pub secure_element: P::SecureElement,
    pub rng: P::Rng,
    pub haptics: P::Haptics,
    pub power: P::Power,
    pub assets: P::Assets,
    pub nfc: P::Nfc,
    pub clock: P::Clock,
}
