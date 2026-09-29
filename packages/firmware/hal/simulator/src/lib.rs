//! Simulator HAL: every peripheral is backed by shared in-memory state.
//!
//! The simulator server holds a [`SimHandle`] to inject input (IMU samples,
//! touches, docking) and read output (framebuffers, haptics, BLE traffic)
//! while the firmware core drives the peripherals exactly as it would on
//! hardware.

pub mod assets;
pub mod imu_script;
pub mod world;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use p256::ecdsa::signature::hazmat::PrehashSigner;
use p256::ecdsa::{Signature, SigningKey};
use sha2::{Digest, Sha256};
use smokebomb_hal::*;

/// Everything the outside world can see or poke.
pub struct SimState {
    /// Frames as last flushed to the "panels".
    pub faces: [FrameBytes; FACE_COUNT],
    /// Frames written but not yet flushed.
    pending: [FrameBytes; FACE_COUNT],
    /// Bumped on every flush so observers can skip unchanged frames.
    pub frame_seq: u64,
    pub brightness: [u8; FACE_COUNT],
    pub display_on: bool,
    /// The 12 V panel supply is paused (for a magnetometer reading).
    pub supply_paused: bool,
    /// How many magnetometer readings the firmware has taken.
    pub mag_reads: u64,
    /// Current resting orientation; returned when the script queue is empty.
    pub imu_resting: ImuSample,
    /// Scripted samples (throws, shakes) consumed one per IMU read.
    pub imu_script: VecDeque<ImuSample>,
    pub touch_mask: u8,
    /// The field the magnetometer reads, in milligauss, in the die's frame,
    /// before the hard-iron offset is added. The server computes it from
    /// the die's pose and the Nest (or a stray magnet) each tick.
    pub mag_field_mg: [i32; 3],
    /// Charger input present, as the power chip reports it: a Nest under
    /// the die, plugged in, with clean contacts, and the charging face
    /// down on its pins.
    pub vbus: bool,
    /// The charger has faulted (thermal, say).
    pub charger_fault: bool,
    /// The Nest's cord is in a live socket.
    pub nest_plugged: bool,
    /// The contacts are dirty: no power gets through, seated or not.
    pub dirty_contacts: bool,
    /// How fast a charging die fills, times 1 %/min.
    pub charge_rate: f32,
    /// Charge gained but not yet a whole percent.
    battery_frac: f32,
    /// Reduced motion, as the browser's setting says.
    pub reduced_motion: bool,
    pub battery_percent: u8,
    /// Local time of day in seconds at clock time zero: the wall clock
    /// reads `(local_base_s + now_ms / 1000) % 86400`.
    pub local_base_s: u32,
    /// The clock's last reading, so the local time can be set to "now".
    last_now_ms: u64,
    pub ble_connected: bool,
    pub ble_tx: VecDeque<Vec<u8>>,
    pub ble_rx: VecDeque<Vec<u8>>,
    pub haptics: VecDeque<HapticEffect>,
    pub power_mode: PowerMode,
    pub nfc_payload: Vec<u8>,
    /// Contents of the simulated 64 MB QSPI flash (only the used prefix).
    pub qspi: Vec<u8>,
    /// When set, the clock reports this instead of wall time (for tests).
    pub manual_time_ms: Option<u64>,
    /// Tests: with manual time, advance it this much on every clock read,
    /// the way a real clock moves on during a tick.
    pub clock_drift_per_read_ms: u64,
    /// Values the RNG returns (as little-endian u32s) before falling back to
    /// OS entropy, so tests can make the die roll a chosen number.
    pub rng_script: VecDeque<u32>,
}

impl Default for SimState {
    fn default() -> Self {
        Self {
            faces: [[0; FRAME_BYTES]; FACE_COUNT],
            pending: [[0; FRAME_BYTES]; FACE_COUNT],
            frame_seq: 0,
            brightness: [255; FACE_COUNT],
            display_on: false,
            supply_paused: false,
            mag_reads: 0,
            imu_resting: imu_script::resting(Face::PosZ),
            imu_script: VecDeque::new(),
            touch_mask: 0,
            mag_field_mg: [0; 3],
            vbus: false,
            charger_fault: false,
            nest_plugged: true,
            dirty_contacts: false,
            charge_rate: 1.0,
            battery_frac: 0.0,
            reduced_motion: false,
            battery_percent: 78,
            local_base_s: 12 * 3600,
            last_now_ms: 0,
            ble_connected: false,
            ble_tx: VecDeque::new(),
            ble_rx: VecDeque::new(),
            haptics: VecDeque::new(),
            power_mode: PowerMode::Normal,
            nfc_payload: Vec::new(),
            qspi: assets::standard_pack(),
            manual_time_ms: None,
            clock_drift_per_read_ms: 0,
            rng_script: VecDeque::new(),
        }
    }
}

impl SimState {
    /// Bring the sensors and the charger up to date with the physical die:
    /// the field the magnetometer feels, power at the contacts, and how far
    /// the battery has charged in `dt` seconds. Call once per tick after
    /// stepping the world.
    pub fn sync_world(&mut self, world: &world::World, dt: f64) {
        self.mag_field_mg = world.mag_field_mg();
        // Power needs the die seated with the charging face down (any of the
        // four rotations), a Nest in a live socket, and clean contacts.
        self.vbus =
            world.seated() && world.face_down() == Face::NegY && self.nest_plugged && !self.dirty_contacts;
        if self.vbus && !self.charger_fault && self.battery_percent < 100 {
            self.battery_frac += self.charge_rate * dt as f32 / 60.0;
            let whole = self.battery_frac.floor();
            self.battery_frac -= whole;
            self.battery_percent = (self.battery_percent as f32 + whole).min(100.0) as u8;
        }
    }

    /// Set the die's local time of day (seconds since midnight) as of now.
    pub fn set_local_time(&mut self, seconds: u32) {
        let up = (self.last_now_ms / 1000) as i64;
        self.local_base_s = (seconds as i64 - up).rem_euclid(86_400) as u32;
    }

    /// Set the battery level (the browser's slider).
    pub fn set_battery(&mut self, percent: u8) {
        self.battery_percent = percent.min(100);
        self.battery_frac = 0.0;
    }
}

/// Cloneable handle to the shared simulator state.
#[derive(Clone, Default)]
pub struct SimHandle(Arc<Mutex<SimState>>);

impl SimHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lock(&self) -> MutexGuard<'_, SimState> {
        // A panic in another holder shouldn't take the simulator down.
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Build the full peripheral set wired to this state.
    pub fn peripherals(&self) -> Peripherals<SimPlatform> {
        Peripherals {
            display: SimDisplay(self.clone()),
            imu: SimImu(self.clone()),
            mag: SimMagnetometer(self.clone()),
            touch: SimTouch(self.clone()),
            ble: SimBle(self.clone()),
            secure_element: SimSecureElement::new(),
            rng: SimRng(Some(self.clone())),
            haptics: SimHaptics(self.clone()),
            power: SimPower(self.clone()),
            assets: SimAssets(self.clone()),
            nfc: SimNfc(self.clone()),
            clock: SimClock {
                handle: self.clone(),
                start: Instant::now(),
            },
        }
    }
}

pub struct SimPlatform;

impl Platform for SimPlatform {
    type Display = SimDisplay;
    type Imu = SimImu;
    type Magnetometer = SimMagnetometer;
    type Touch = SimTouch;
    type Ble = SimBle;
    type SecureElement = SimSecureElement;
    type Rng = SimRng;
    type Haptics = SimHaptics;
    type Power = SimPower;
    type Assets = SimAssets;
    type Nfc = SimNfc;
    type Clock = SimClock;
}

pub struct SimDisplay(SimHandle);

impl Display for SimDisplay {
    fn write_frame(&mut self, face: Face, frame: &FrameBytes) -> HalResult<()> {
        self.0.lock().pending[face.index()] = *frame;
        Ok(())
    }

    fn flush(&mut self) -> HalResult<()> {
        let mut s = self.0.lock();
        if s.faces != s.pending {
            s.faces = s.pending;
            s.frame_seq += 1;
        }
        Ok(())
    }

    fn set_brightness(&mut self, face: Face, level: u8) -> HalResult<()> {
        self.0.lock().brightness[face.index()] = level;
        Ok(())
    }

    fn set_enabled(&mut self, enabled: bool) -> HalResult<()> {
        self.0.lock().display_on = enabled;
        Ok(())
    }

    fn set_supply_paused(&mut self, paused: bool) -> HalResult<()> {
        self.0.lock().supply_paused = paused;
        Ok(())
    }
}

pub struct SimImu(SimHandle);

impl Imu for SimImu {
    fn read(&mut self) -> HalResult<Option<ImuSample>> {
        let mut s = self.0.lock();
        let sample = s.imu_script.pop_front().unwrap_or(s.imu_resting);
        Ok(Some(sample))
    }
}

/// What the die's own steel, magnets and haptic motor add to every
/// magnetometer reading, in milligauss (die frame). The firmware subtracts
/// it as the factory calibration.
pub const SIM_HARD_IRON_MG: [i32; 3] = [820, -340, 510];

pub struct SimMagnetometer(SimHandle);

impl Magnetometer for SimMagnetometer {
    fn read(&mut self) -> HalResult<[i32; 3]> {
        let mut s = self.0.lock();
        s.mag_reads += 1;
        Ok([0, 1, 2].map(|i| s.mag_field_mg[i] + SIM_HARD_IRON_MG[i]))
    }

    fn hard_iron(&self) -> [i32; 3] {
        SIM_HARD_IRON_MG
    }
}

pub struct SimTouch(SimHandle);

impl Touch for SimTouch {
    fn read(&mut self) -> HalResult<u8> {
        Ok(self.0.lock().touch_mask)
    }
}

pub struct SimBle(SimHandle);

impl Ble for SimBle {
    fn is_connected(&self) -> bool {
        self.0.lock().ble_connected
    }

    fn send(&mut self, payload: &[u8]) -> HalResult<()> {
        let mut s = self.0.lock();
        if !s.ble_connected {
            return Err(HalError::NotReady);
        }
        s.ble_tx.push_back(payload.to_vec());
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> HalResult<Option<usize>> {
        let Some(msg) = self.0.lock().ble_rx.pop_front() else {
            return Ok(None);
        };
        let n = msg.len().min(buf.len());
        buf[..n].copy_from_slice(&msg[..n]);
        Ok(Some(n))
    }

    fn set_advertising(&mut self, _enabled: bool) -> HalResult<()> {
        Ok(())
    }
}

/// Software stand-in for the ATECC608 with a fixed, publicly known dev key.
/// Rolls signed by it are verifiable but obviously not trustworthy; the
/// server should only accept this key in development.
pub struct SimSecureElement {
    key: SigningKey,
    counter: u32,
}

/// Serial reported by every simulated die.
pub const SIM_SERIAL: [u8; 9] = [0x01, 0x23, 0x5B, 0x0E, 0x00, 0x00, 0x00, 0x00, 0xEE];

impl Default for SimSecureElement {
    fn default() -> Self {
        Self::new()
    }
}

impl SimSecureElement {
    pub fn new() -> Self {
        let seed = Sha256::digest(b"smokebomb simulator dev key v1");
        let key = SigningKey::from_slice(&seed).expect("sha256 output is a valid P-256 scalar");
        Self { key, counter: 0 }
    }
}

impl SecureElement for SimSecureElement {
    fn serial(&mut self) -> HalResult<[u8; 9]> {
        Ok(SIM_SERIAL)
    }

    fn public_key(&mut self) -> HalResult<[u8; 64]> {
        let point = self.key.verifying_key().to_encoded_point(false);
        let mut out = [0u8; 64];
        out.copy_from_slice(&point.as_bytes()[1..]);
        Ok(out)
    }

    fn sign_digest(&mut self, digest: &[u8; 32]) -> HalResult<[u8; 64]> {
        let sig: Signature = self.key.sign_prehash(digest).map_err(|_| HalError::Bus)?;
        Ok(sig.to_bytes().into())
    }

    fn next_counter(&mut self) -> HalResult<u32> {
        let c = self.counter;
        self.counter += 1;
        Ok(c)
    }
}

/// OS entropy, or scripted values from [`SimState::rng_script`] when wired
/// to a [`SimHandle`]. `SimRng::default()` is plain OS entropy.
#[derive(Default)]
pub struct SimRng(pub Option<SimHandle>);

impl Rng for SimRng {
    fn fill_bytes(&mut self, buf: &mut [u8]) -> HalResult<()> {
        if let Some(h) = &self.0 {
            let mut s = h.lock();
            if buf.len() == 4 {
                if let Some(v) = s.rng_script.pop_front() {
                    buf.copy_from_slice(&v.to_le_bytes());
                    return Ok(());
                }
            }
        }
        getrandom::getrandom(buf).map_err(|_| HalError::NotReady)
    }
}

pub struct SimHaptics(SimHandle);

impl Haptics for SimHaptics {
    fn play(&mut self, effect: HapticEffect) -> HalResult<()> {
        self.0.lock().haptics.push_back(effect);
        Ok(())
    }
}

pub struct SimPower(SimHandle);

impl Power for SimPower {
    fn battery(&mut self) -> HalResult<BatteryStatus> {
        let s = self.0.lock();
        Ok(BatteryStatus {
            percent: s.battery_percent,
            millivolts: 3_700 + s.battery_percent as u16 * 5,
            vbus: s.vbus,
            charge: match (s.vbus, s.charger_fault, s.battery_percent) {
                (false, _, _) => ChargeState::Idle,
                (true, true, _) => ChargeState::Fault,
                (true, false, 100..) => ChargeState::Full,
                (true, false, _) => ChargeState::Charging,
            },
        })
    }

    fn set_mode(&mut self, mode: PowerMode) -> HalResult<()> {
        self.0.lock().power_mode = mode;
        Ok(())
    }
}

pub struct SimAssets(SimHandle);

impl AssetStore for SimAssets {
    fn capacity(&self) -> u32 {
        smokebomb_shared::assets::QSPI_CAPACITY
    }

    fn read(&mut self, offset: u32, buf: &mut [u8]) -> HalResult<()> {
        let s = self.0.lock();
        let start = offset as usize;
        let end = start + buf.len();
        if end > self.capacity() as usize {
            return Err(HalError::InvalidArgument);
        }
        // Erased NOR flash reads as 0xFF beyond what has been written.
        buf.fill(0xFF);
        if start < s.qspi.len() {
            let avail = end.min(s.qspi.len());
            buf[..avail - start].copy_from_slice(&s.qspi[start..avail]);
        }
        Ok(())
    }
}

pub struct SimNfc(SimHandle);

impl Nfc for SimNfc {
    fn set_payload(&mut self, ndef: &[u8]) -> HalResult<()> {
        self.0.lock().nfc_payload = ndef.to_vec();
        Ok(())
    }
}

pub struct SimClock {
    handle: SimHandle,
    start: Instant,
}

impl Clock for SimClock {
    fn now_ms(&self) -> u64 {
        let mut s = self.handle.lock();
        let drift = s.clock_drift_per_read_ms;
        let now = match &mut s.manual_time_ms {
            Some(t) => {
                *t += drift;
                *t
            }
            None => self.start.elapsed().as_millis() as u64,
        };
        s.last_now_ms = now;
        now
    }

    fn local_seconds(&self) -> u32 {
        let now = self.now_ms();
        let base = self.handle.lock().local_base_s as u64;
        ((base + now / 1000) % 86_400) as u32
    }
}
