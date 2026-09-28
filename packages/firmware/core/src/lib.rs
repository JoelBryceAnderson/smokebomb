//! Smokebomb firmware core.
//!
//! Pure `no_std` logic with no knowledge of the chip it runs on. Everything
//! hardware-facing goes through [`smokebomb_hal`] traits, so this crate runs
//! unchanged on the nRF54L15 and inside the desktop simulator.
//!
//! The runner (Zephyr thread on hardware, tokio task in the simulator) calls
//! [`Firmware::tick`] at [`TICK_HZ`].

#![no_std]

pub mod animation;
pub mod display;
pub mod menu;
pub mod motion;
pub mod orientation;
pub mod roll;
pub mod state;

use heapless::Vec;
use smokebomb_hal::{
    Ble, Clock, Display, Face, FrameBytes, HalResult, Haptics, Imu, Peripherals, Platform, Power, Touch,
    FRAME_BYTES,
};
use smokebomb_shared::{DieKind, SignedRoll};

use animation::AnimationPlayer;
use display::Framebuffer;
use menu::Settings;
use motion::MotionDetector;
use orientation::TextOrientation;
use roll::RollEngine;
use state::{Command, Event, Mode, StateMachine};

/// Main loop rate. The mockup animates at display rate (~60 Hz); panels
/// accept at most 100 Hz (SIM_SPEC B1).
pub const TICK_HZ: u32 = 60;

/// Low-pass factor for the gravity estimate, per tick.
const UP_FILTER: f32 = 0.25;

/// How long a face must be held to open the menu.
pub const MENU_HOLD_MS: u64 = 1_500;

pub struct Firmware<P: Platform> {
    hw: Peripherals<P>,
    sm: StateMachine,
    motion: MotionDetector,
    settings: Settings,
    roller: RollEngine,
    player: AnimationPlayer,
    frames: [Framebuffer; smokebomb_hal::FACE_COUNT],
    /// Packed 4bpp scratch for the panel and for streaming assets.
    panel: FrameBytes,
    /// Filtered gravity-up direction in die coordinates (milli-g).
    up: [f32; 3],
    orientation: TextOrientation,
    touch_since: Option<u64>,
    menu_hold_fired: bool,
    up_face: Face,
    last_roll: Option<SignedRoll>,
    docked: bool,
}

impl<P: Platform> Firmware<P> {
    pub fn new(mut hw: Peripherals<P>) -> HalResult<Self> {
        let player = AnimationPlayer::load(&mut hw.assets);
        let roller = RollEngine::new(&mut hw.secure_element)?;
        hw.display.set_enabled(true)?;
        Ok(Self {
            hw,
            sm: StateMachine::new(),
            motion: MotionDetector::new(),
            settings: Settings::default(),
            roller,
            player,
            frames: [Framebuffer::new(); smokebomb_hal::FACE_COUNT],
            panel: [0; FRAME_BYTES],
            up: [0.0, 0.0, 1000.0],
            orientation: TextOrientation::new(),
            touch_since: None,
            menu_hold_fired: false,
            up_face: Face::PosZ,
            last_roll: None,
            docked: false,
        })
    }

    pub fn mode(&self) -> &Mode {
        self.sm.mode()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn last_roll(&self) -> Option<&SignedRoll> {
        self.last_roll.as_ref()
    }

    pub fn frames(&self) -> &[Framebuffer; smokebomb_hal::FACE_COUNT] {
        &self.frames
    }

    pub fn orientation(&self) -> &TextOrientation {
        &self.orientation
    }

    /// One pass of the main loop: sample inputs, advance the state machine,
    /// execute its commands, render and present.
    pub fn tick(&mut self) -> HalResult<()> {
        let now = self.hw.clock.now_ms();
        let mut events: Vec<Event, 8> = Vec::new();

        if let Some(sample) = self.hw.imu.read()? {
            let raw = orientation::up_from(&sample);
            for (u, r) in self.up.iter_mut().zip(raw) {
                *u += (r - *u) * UP_FILTER;
            }
            self.orientation.update(self.up);
            if let Some(face) = motion::up_face(&sample) {
                self.up_face = face;
            }
            if let Some(m) = self.motion.update(&sample, now) {
                let _ = events.push(Event::Motion(m));
            }
        }

        self.poll_touch(now, &mut events)?;

        let battery = self.hw.power.battery()?;
        if battery.docked != self.docked {
            self.docked = battery.docked;
            let _ = events.push(Event::Docked(battery.docked));
        }

        // BLE: drain writes from the phone. Decoding into `PhoneToDie` and
        // acting on it is not wired up yet.
        let mut rx = [0u8; 64];
        while self.hw.ble.receive(&mut rx)?.is_some() {}

        let _ = events.push(Event::Tick);
        for event in events {
            let commands = self.sm.handle(event, now);
            for cmd in commands {
                self.execute(cmd)?;
            }
        }

        self.render(now)?;
        Ok(())
    }

    fn poll_touch(&mut self, now: u64, events: &mut Vec<Event, 8>) -> HalResult<()> {
        let mask = self.hw.touch.read()?;
        // Grip rejection: touches while the die is moving are ignored.
        let usable = mask != 0 && self.motion.is_still();
        match (usable, self.touch_since) {
            (true, None) => {
                self.touch_since = Some(now);
                self.menu_hold_fired = false;
            }
            (true, Some(since)) if !self.menu_hold_fired && now - since >= MENU_HOLD_MS => {
                self.menu_hold_fired = true;
                let _ = events.push(Event::LongPress);
            }
            (false, Some(_)) => {
                if !self.menu_hold_fired {
                    let _ = events.push(Event::Tap);
                }
                self.touch_since = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn execute(&mut self, cmd: Command) -> HalResult<()> {
        match cmd {
            Command::PlayClip(clip) => self.player.play(clip),
            Command::StopClip => self.player.stop(),
            Command::Haptic(effect) => self.hw.haptics.play(effect)?,
            Command::Roll => {
                let signed = self.roller.roll(
                    &mut self.hw.rng,
                    &mut self.hw.secure_element,
                    self.settings.die,
                    self.settings.count,
                    self.hw.clock.now_ms(),
                )?;
                if is_max(&signed, self.settings.die) {
                    self.player.play(smokebomb_shared::assets::ClipId::MaxBurst);
                    self.hw
                        .haptics
                        .play(smokebomb_hal::HapticEffect::MaxCelebration)?;
                }
                if self.hw.ble.is_connected() {
                    // TODO: frame + serialise `DieToPhone::Roll` (postcard) once the
                    // GATT protocol is pinned down. Send the raw digest for now.
                    self.hw.ble.send(&signed.record.digest())?;
                }
                self.last_roll = Some(signed);
            }
            Command::MenuInput(input) => self.settings.apply(input),
        }
        Ok(())
    }

    fn render(&mut self, now: u64) -> HalResult<()> {
        let animating = self
            .player
            .render(&mut self.hw.assets, &mut self.frames, &mut self.panel, now)?;
        if !animating {
            for fb in self.frames.iter_mut() {
                fb.clear();
            }
            match self.sm.mode() {
                Mode::Reveal { .. } => {
                    if let Some(roll) = &self.last_roll {
                        display::draw_result(&mut self.frames, &self.orientation, self.up_face, &roll.record);
                    }
                }
                Mode::Menu(page) => {
                    let face = self.up_face;
                    menu::render(
                        &mut self.frames[face.index()],
                        *page,
                        &self.settings,
                        self.orientation.quarter(face),
                    )
                }
                Mode::Nest => {
                    // TODO: analog clock + pixel shifting for burn-in mitigation.
                    let pct = self.hw.power.battery()?.percent;
                    let face = self.up_face;
                    let rot = self.orientation.quarter(face);
                    self.frames[face.index()].draw_number(pct as u16, 6, display::FG, rot);
                }
                _ => {}
            }
        }
        for face in Face::ALL {
            self.frames[face.index()].quantize(&mut self.panel);
            self.hw.display.write_frame(face, &self.panel)?;
        }
        self.hw.display.flush()
    }
}

/// Every die shows its top value (N > 2); Pass the Pot has no max (SIM_SPEC C6).
fn is_max(roll: &SignedRoll, die: DieKind) -> bool {
    die.is_numeric()
        && die.sides() > 2
        && !roll.record.values.is_empty()
        && roll.record.values.iter().all(|&v| v == die.sides())
}
