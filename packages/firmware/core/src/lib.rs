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
pub mod font;
pub mod gfx;
pub mod menu;
pub mod motion;
pub mod orientation;
pub mod pack;
pub mod potato;
pub mod roll;
pub mod screens;
pub mod state;
pub mod tips;
pub mod ui;

use heapless::Vec;
use smokebomb_hal::{
    Ble, Clock, Display, Face, FrameBytes, HalResult, Haptics, Imu, Peripherals, Platform, Power, Rng, Touch,
    FRAME_BYTES,
};
use smokebomb_shared::{DieKind, SignedRoll};

use animation::AnimationPlayer;
use display::Framebuffer;
use font::Fonts;
use gfx::{Layer, Painter, Transform};
use menu::{Draft, Settings};
use motion::{Motion, MotionDetector};
use orientation::TextOrientation;
use pack::PackIndex;
use potato::Potato;
use roll::RollEngine;
use screens::Ctx;
use state::{Command, Event, Mode, StateMachine};
use tips::{Frame, TipDir, TipTracker, TipUpdate};
use ui::{FaceContent, Ui};

/// Main loop rate. The mockup animates at display rate (~60 Hz); panels
/// accept at most 100 Hz (SIM_SPEC B1).
pub const TICK_HZ: u32 = 60;

/// Low-pass factor for the gravity estimate, per tick.
const UP_FILTER: f32 = 0.25;

/// How long a screen must be held to open the menu (SIM_SPEC C3).
pub const MENU_HOLD_MS: u64 = 800;
/// The hold ring appears once a touch is clearly a hold, not a tap.
pub const HOLD_RING_AFTER_MS: u64 = 220;
/// The menu closes without saving after this long without a tip.
pub const MENU_IDLE_MS: u64 = 25_000;

/// The open menu: its draft, which way the person is holding the die, and
/// the tip being made.
struct MenuSession {
    draft: Draft,
    frame: Frame,
    tips: TipTracker,
    last_input: u64,
    turning: Option<(TipDir, f32)>,
}

pub struct Firmware<P: Platform> {
    hw: Peripherals<P>,
    sm: StateMachine,
    motion: MotionDetector,
    settings: Settings,
    potato: Potato,
    roller: RollEngine,
    player: AnimationPlayer,
    fonts: Fonts,
    ui: Ui,
    frames: [Framebuffer; smokebomb_hal::FACE_COUNT],
    layer: Layer,
    /// Packed 4bpp scratch for the panel and for streaming assets.
    panel: FrameBytes,
    /// Filtered gravity-up direction in die coordinates (milli-g); `None`
    /// until the first IMU sample.
    up: Option<[f32; 3]>,
    orientation: TextOrientation,
    touch_since: Option<u64>,
    /// The face a touch started on (the lowest one, if several).
    touch_face: Face,
    menu_hold_fired: bool,
    menu: Option<MenuSession>,
    last_imu_ms: Option<u64>,
    up_face: Face,
    last_roll: Option<SignedRoll>,
    docked: bool,
}

impl<P: Platform> Firmware<P> {
    pub fn new(mut hw: Peripherals<P>) -> HalResult<Self> {
        let pack = PackIndex::load(&mut hw.assets);
        let player = AnimationPlayer::load(&mut hw.assets, &pack);
        let fonts = Fonts::load(&mut hw.assets, &pack);
        let roller = RollEngine::new(&mut hw.secure_element)?;
        hw.display.set_enabled(true)?;
        Ok(Self {
            hw,
            sm: StateMachine::new(),
            motion: MotionDetector::new(),
            settings: Settings::default(),
            potato: Potato::new(),
            roller,
            player,
            fonts,
            ui: Ui::new(),
            frames: [Framebuffer::new(); smokebomb_hal::FACE_COUNT],
            layer: Layer::new(),
            panel: [0; FRAME_BYTES],
            up: None,
            orientation: TextOrientation::new(),
            touch_since: None,
            touch_face: Face::PosZ,
            menu_hold_fired: false,
            menu: None,
            last_imu_ms: None,
            up_face: Face::PosY,
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

    /// Replaces the settings, as when they're loaded from flash at boot.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.apply_settings();
    }

    /// The saved mode decides whether throws roll or belong to a game.
    fn apply_settings(&mut self) {
        self.sm.set_rolls(self.settings.play().rolls());
        self.potato = Potato::new();
    }

    /// The Hot Potato round in play, if any.
    pub fn potato(&self) -> &Potato {
        &self.potato
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

    pub fn booting(&self) -> bool {
        self.ui.booting()
    }

    /// The face the open menu is on, if it's open. The simulator turns this
    /// face toward the viewer when the menu opens.
    pub fn menu_front(&self) -> Option<Face> {
        self.menu.as_ref().map(|m| m.frame.front_face())
    }

    /// The menu's draft, if it's open.
    pub fn menu_draft(&self) -> Option<&Draft> {
        self.menu.as_ref().map(|m| &m.draft)
    }

    /// One pass of the main loop: sample inputs, advance the state machine,
    /// execute its commands, render and present.
    pub fn tick(&mut self) -> HalResult<()> {
        let now = self.hw.clock.now_ms();
        let mut events: Vec<Event, 8> = Vec::new();

        if let Some(sample) = self.hw.imu.read()? {
            let raw = orientation::up_from(&sample);
            let up = self.up.get_or_insert(raw);
            for (u, r) in up.iter_mut().zip(raw) {
                *u += (r - *u) * UP_FILTER;
            }
            self.orientation.update(*up);
            if let Some(face) = motion::up_face(&sample) {
                self.up_face = face;
            }
            if let Some(m) = self.motion.update(&sample, now) {
                let _ = events.push(Event::Motion(m));
            }
            let dt = self.last_imu_ms.map_or(0, |t| now - t) as f32 / 1000.0;
            self.last_imu_ms = Some(now);
            self.menu_tips(&sample, dt, now)?;
        }
        if let Some(m) = &self.menu {
            if m.turning.is_none() && now - m.last_input >= MENU_IDLE_MS {
                let _ = events.push(Event::MenuTimeout);
            }
        }

        if self.poll_touch(now, &mut events)? {
            self.ui.touch(now);
        }

        let battery = self.hw.power.battery()?;
        if battery.docked != self.docked {
            self.docked = battery.docked;
            let _ = events.push(Event::Docked(battery.docked));
        }

        // BLE: drain writes from the phone. Decoding into `PhoneToDie` and
        // acting on it is not wired up yet.
        let mut rx = [0u8; 64];
        while self.hw.ble.receive(&mut rx)?.is_some() {}

        let before = *self.sm.mode();
        let _ = events.push(Event::Tick);
        let game = !self.settings.play().rolls();
        for event in events {
            if game && !matches!(self.sm.mode(), Mode::Menu | Mode::Nest) {
                match event {
                    Event::Motion(Motion::Shaking) if self.potato.is_idle() => self.light_potato(now)?,
                    Event::Tap => {
                        for cmd in self.potato.tap(now) {
                            self.execute(cmd)?;
                        }
                    }
                    // No opening the menu mid-round.
                    Event::LongPress if self.potato.is_lit() => continue,
                    _ => {}
                }
            }
            if event == Event::Docked(true) {
                for cmd in self.potato.reset() {
                    self.execute(cmd)?;
                }
            }
            if event == Event::Tap {
                self.ui.tap(now, self.sm.mode());
            }
            let commands = self.sm.handle(event, now);
            for cmd in commands {
                self.execute(cmd)?;
            }
        }
        for cmd in self.potato.tick(now) {
            self.execute(cmd)?;
        }
        let after = *self.sm.mode();
        self.ui.tick(now, &before, &after, self.up_face, self.docked);

        self.render(now)?;
        Ok(())
    }

    /// A shake lit Hot Potato: pick the fuse at random within the setting.
    fn light_potato(&mut self, now: u64) -> HalResult<()> {
        let range = self.settings.fuse.range_ms();
        let fuse = potato::fuse_from(self.hw.rng.next_u32()?, range);
        self.ui.game_started();
        for cmd in self.potato.light(now, fuse, range.1) {
            self.execute(cmd)?;
        }
        Ok(())
    }

    /// Feed the gyro to the menu's tip tracker and apply finished tips.
    fn menu_tips(&mut self, sample: &smokebomb_hal::ImuSample, dt: f32, now: u64) -> HalResult<()> {
        let Some(m) = &mut self.menu else {
            return Ok(());
        };
        let (armed, update) = m.tips.update(sample.gyro_mdps, dt, now, &m.frame);
        if armed {
            // The die is still in front of the person: take the sky from
            // here (the simulator has just turned the die toward the viewer).
            let up = self.up.unwrap_or([0.0, 1.0, 0.0]);
            m.frame = Frame::new(m.frame.front_face(), up);
        }
        match update {
            TipUpdate::Turning { dir, progress } => {
                if m.turning.is_none() {
                    self.hw.haptics.play(smokebomb_hal::HapticEffect::MenuTip)?;
                    m.last_input = now;
                }
                m.turning = Some((dir, progress));
            }
            TipUpdate::Done(dir) => {
                m.draft = m.draft.tipped(dir);
                m.frame = m.frame.after(dir);
                m.turning = None;
                m.last_input = now;
            }
            TipUpdate::Cancelled => m.turning = None,
            TipUpdate::None => {}
        }
        Ok(())
    }

    /// Returns true when a touch starts.
    fn poll_touch(&mut self, now: u64, events: &mut Vec<Event, 8>) -> HalResult<bool> {
        let mask = self.hw.touch.read()?;
        // Grip rejection: touches while the die is moving are ignored. In
        // the menu the die is in the hand anyway; only a tip in progress
        // blocks a hold.
        let usable = mask != 0
            && match &self.menu {
                Some(m) => m.turning.is_none(),
                None => self.motion.is_still(),
            };
        let mut started = false;
        match (usable, self.touch_since) {
            (true, None) => {
                self.touch_since = Some(now);
                self.touch_face = Face::ALL[mask.trailing_zeros() as usize % Face::ALL.len()];
                self.menu_hold_fired = false;
                started = true;
            }
            // Held for more than 0.8 s: at 60 Hz that's 49 frames, as in the
            // mockup, whose float clock never quite reaches 0.8 after 48.
            (true, Some(since)) if !self.menu_hold_fired && now - since > MENU_HOLD_MS => {
                self.menu_hold_fired = true;
                let _ = events.push(Event::LongPress);
            }
            (false, Some(_)) => {
                // Letting go after a hold that saved the menu shows the
                // setup like a tap does (the mockup's pointer-up); letting go
                // after the hold that opened it doesn't.
                if !self.menu_hold_fired || self.menu.is_none() {
                    let _ = events.push(Event::Tap);
                }
                self.touch_since = None;
            }
            _ => {}
        }
        Ok(started)
    }

    /// How far the current touch is toward a hold (0–1), once the ring shows.
    fn hold_progress(&self, now: u64) -> Option<f32> {
        let since = self.touch_since?;
        let held = now - since;
        let ring = !self.menu_hold_fired
            && !self.potato.is_lit()
            && held > HOLD_RING_AFTER_MS
            && matches!(self.sm.mode(), Mode::Idle | Mode::Reveal { .. } | Mode::Menu);
        ring.then(|| {
            ((held - HOLD_RING_AFTER_MS) as f32 / (MENU_HOLD_MS - HOLD_RING_AFTER_MS) as f32).min(1.0)
        })
    }

    fn execute(&mut self, cmd: Command) -> HalResult<()> {
        let now = self.hw.clock.now_ms();
        match cmd {
            Command::PlayClip(clip) => self.player.play(clip),
            Command::StopClip => self.player.stop(),
            Command::Haptic(effect) => self.hw.haptics.play(effect)?,
            Command::Roll => {
                let (die, count) = self.settings.active();
                let signed = self.roller.roll(
                    &mut self.hw.rng,
                    &mut self.hw.secure_element,
                    die,
                    count,
                    self.hw.clock.now_ms(),
                )?;
                if is_max(&signed, die) {
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
            Command::MenuOpen => {
                self.potato = Potato::new();
                let up = self.up.unwrap_or([0.0, 1.0, 0.0]);
                self.menu = Some(MenuSession {
                    draft: Draft::new(&self.settings),
                    frame: Frame::new(self.touch_face, up),
                    // Starts disarmed: tips count once the die is still.
                    tips: TipTracker::new(),
                    last_input: now,
                    turning: None,
                });
                self.ui.menu_opened(now);
            }
            Command::MenuClose { save } => {
                if let Some(m) = self.menu.take() {
                    if save && m.draft.restart_selected() {
                        self.player.stop();
                        self.ui.restart(now);
                    } else {
                        if save {
                            m.draft.commit(&mut self.settings);
                            self.apply_settings();
                        }
                        self.ui.menu_closed(now, m.frame.front_face(), m.draft, save);
                    }
                }
            }
        }
        Ok(())
    }

    fn render(&mut self, now: u64) -> HalResult<()> {
        let mode = *self.sm.mode();
        let up = self.up_face;
        let label = self.settings.setup().label();
        let battery = self.hw.power.battery()?.percent;
        let potato_view = if matches!(mode, Mode::Menu | Mode::Nest) {
            None
        } else if let Some(t) = self.potato.boomed_for(now) {
            Some(PotatoView::Boom(t as f32 / 1000.0))
        } else if self.potato.is_lit() {
            Some(PotatoView::Fuse(self.potato.heat(now), self.potato.pulse(now)))
        } else {
            None
        };
        let hold = self.hold_progress(now).map(|p| (self.touch_face, p));
        let Self {
            frames,
            layer,
            fonts,
            hw,
            ui,
            orientation,
            last_roll,
            menu,
            ..
        } = self;
        let record = last_roll.as_ref().map(|r| &r.record);
        let blackout = ui.blackout();

        for face in Face::ALL {
            let fb = &mut frames[face.index()];
            fb.clear();
            if blackout {
                continue;
            }
            let content = ui.content(now, face, up, &mode, record.is_some());
            // A round of Hot Potato takes the faces, except the one facing
            // down (H2).
            let potato_face = potato_view.filter(|_| face != up.opposite());
            let content = if potato_face.is_some() {
                FaceContent::Blank
            } else {
                content
            };
            let rot = orientation.quarter(face);
            let mut painter = Painter::new(fb, layer, Transform::quarter(rot));
            let mut c = Ctx {
                painter: &mut painter,
                fonts,
                assets: &mut hw.assets,
            };

            // Under the content: the save flash (not on the face-down
            // screen, H2), the closed menu fading, and the hold ring.
            if face != up.opposite() {
                screens::draw_flash(&mut c, ui.flash(now));
            }
            if let Some((draft, u)) = ui.menu_fade(now, face) {
                screens::draw_menu(
                    &mut c,
                    &draft,
                    battery as f32 / 100.0,
                    0.0,
                    0.0,
                    1.0 - u,
                    1.0 + 0.12 * u,
                );
                screens::draw_hold_ring(&mut c, 1.0, 1.0 - u, u * 3.0);
            }
            if menu.is_none() {
                if let Some((f, p)) = hold {
                    if f == face {
                        screens::draw_hold_ring(&mut c, p, 1.0, 0.0);
                    }
                }
            }

            match content {
                FaceContent::Blank => {}
                FaceContent::Boot { t, top } => screens::draw_boot(&mut c, face.index(), top, t),
                FaceContent::Wake { alpha } => screens::draw_wake_label(&mut c, &label, alpha),
                FaceContent::Result { alpha } => {
                    if let Some(r) = record {
                        screens::draw_result(&mut c, r, display::FG, alpha);
                    }
                }
                FaceContent::Success { t, setup } => {
                    screens::draw_success(&mut c, &setup.label(), setup.nudge(), t);
                }
                FaceContent::Menu => {
                    if let Some(m) = menu {
                        draw_menu_face(&mut c, m, ui, face, now, battery, hold.map(|(_, p)| p));
                    }
                }
                // Placeholder until the Nest screens are built.
                FaceContent::Nest if face == up => {
                    fb_of(c.painter).draw_number(battery as u16, 6, display::FG, rot);
                }
                FaceContent::Nest => {}
            }
            match potato_face {
                Some(PotatoView::Fuse(heat, pulse)) => screens::draw_fuse(&mut c, heat, pulse),
                Some(PotatoView::Boom(t)) => screens::draw_boom(&mut c, t),
                None => {}
            }
        }

        // Placeholder smoke on top, until the particle system lands.
        if !self.ui.booting() {
            self.player
                .render(&mut self.hw.assets, &mut self.frames, &mut self.panel, now)?;
        }
        for face in Face::ALL {
            self.frames[face.index()].quantize(&mut self.panel);
            self.hw.display.write_frame(face, &self.panel)?;
        }
        self.hw.display.flush()
    }
}

/// What a round of Hot Potato shows on the faces.
#[derive(Clone, Copy)]
enum PotatoView {
    /// Heat and pulse, both 0–1.
    Fuse(f32, f32),
    /// Seconds since it went off.
    Boom(f32),
}

/// One face while the menu is open (C3). The menu shows on the front face;
/// during a tip the old page slides off against the turn and fades while
/// the new one slides in from the leading edge of the face coming round.
fn draw_menu_face<A: smokebomb_hal::AssetStore>(
    c: &mut Ctx<A>,
    m: &MenuSession,
    ui: &Ui,
    face: Face,
    now: u64,
    battery: u8,
    hold: Option<f32>,
) {
    let battery = battery as f32 / 100.0;
    let front = m.frame.front_face();
    if let Some((dir, u)) = m.turning {
        let (mx, my) = m.frame.motion_dir(face, dir);
        let d = screens::TIP_SLIDE;
        if face == front {
            screens::draw_menu(c, &m.draft, battery, -mx * u * d, -my * u * d, 1.0 - u, 1.0);
        }
        if face == m.frame.next_front(dir) {
            let next = m.draft.tipped(dir);
            let (ox, oy) = (mx * (1.0 - u) * d, my * (1.0 - u) * d);
            screens::draw_menu(c, &next, battery, ox, oy, u, 1.0);
        }
        return;
    }
    if face == front {
        let intro = ui.menu_intro(now);
        screens::draw_hold_ring(c, 1.0, intro.ring_alpha, intro.ring_grow);
        screens::draw_menu(c, &m.draft, battery, 0.0, 0.0, intro.alpha, intro.scale);
        if let Some(p) = hold {
            screens::draw_hold_ring(c, p, 1.0, 0.0);
        }
    }
}

fn fb_of<'a>(painter: &'a mut Painter<'_>) -> &'a mut Framebuffer {
    painter.framebuffer()
}

/// Every die shows its top value (N > 2); Pass the Pot has no max (SIM_SPEC C6).
fn is_max(roll: &SignedRoll, die: DieKind) -> bool {
    die.is_numeric()
        && die.sides() > 2
        && !roll.record.values.is_empty()
        && roll.record.values.iter().all(|&v| v == die.sides())
}
