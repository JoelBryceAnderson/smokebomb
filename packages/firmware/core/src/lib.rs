//! Sugarcube firmware core.
//!
//! Pure `no_std` logic with no knowledge of the chip it runs on. Everything
//! hardware-facing goes through [`smokebomb_hal`] traits, so this crate runs
//! unchanged on the nRF54L15 and inside the desktop simulator.
//!
//! The runner (Zephyr thread on hardware, tokio task in the simulator) calls
//! [`Firmware::tick`] at [`TICK_HZ`].

#![no_std]

#[cfg(test)]
extern crate std;

pub mod apps;
pub mod bench;
pub mod display;
pub mod effects;
pub mod font;
pub mod font64;
pub mod gesture;
pub mod gfx;
pub mod icons;
pub mod menu;
pub mod motion;
pub mod nest;
pub mod numerals64;
pub mod orientation;
pub mod pack;
pub mod palette64;
pub mod panel;
pub mod pigfx;
pub mod pigs;
pub mod potato;
pub mod roll;
pub mod screens;
pub mod screens64;
pub mod smoke;
pub mod sprites64;
pub mod state;
pub mod table;
pub mod target;
pub mod tier64;
pub mod tiers;
pub mod tips;
pub mod ui;

use core::mem::MaybeUninit;
use core::ptr::addr_of_mut;

use heapless::Vec;
use smokebomb_hal::{
    Ble, Clock, Display, Face, HalResult, Haptics, Imu, Magnetometer, Peripherals, Platform, Power, Rng,
    SecureElement, Target, TargetOf, Touch,
};
use smokebomb_shared::protocol::Inventory;
use smokebomb_shared::{DieKind, ModeId, ModeSet, SignedRoll};

use apps::{Action, App, Apps, Effect, Effects, MotionUse, Pending, SmokeOp, Tap, Throw};
use display::Framebuffer;
use font::Fonts;
use gesture::{Gesture, Gestures};
use gfx::{Layer, Painter, Transform};
use menu::{Draft, Held, Page, PlayMode, Settings};
use motion::{Motion, MotionDetector};
use nest::{Nest, NestFace};
use orientation::TextOrientation;
use pack::PackIndex;
use pigs::Pigs;
use potato::Potato;
use roll::RollEngine;
use screens::Ctx;
use smoke::{Smoke, Special};
use state::{Command, Event, Mode, StateMachine};
use target::{Delivery, DisplayTarget};
use tips::{Frame, TipDir, TipTracker, TipUpdate};
use ui::{FaceContent, Ui};

/// At or below this the die suggests Low power mode, and shows the charge
/// glyph on its charging face.
pub const LOW_BATTERY_PCT: u8 = 15;

/// Main loop rate. The mockup animates at display rate (~60 Hz); panels
/// accept at most 100 Hz (SIM_SPEC B1).
pub const TICK_HZ: u32 = 60;

/// Low-pass factor for the gravity estimate, per tick.
const UP_FILTER: f32 = 0.25;

/// The IMU delivers one sample per tick at its output data rate (the
/// simulator likewise produces one per tick). Gyro rates are integrated over
/// this sample period rather than the wall clock, so a late or bunched tick
/// doesn't lose rotation.
const IMU_SAMPLE_S: f32 = 1.0 / TICK_HZ as f32;

/// How long the lock-in stays up before the screens go quiet.
pub const LOCKED_MS: u64 = 12_000;
/// The menu closes without saving after this long without a tip.
pub const MENU_IDLE_MS: u64 = 25_000;

/// How the screens were oriented when a result was revealed. While the
/// result lasts they stay that way, like a printed die: picking the die up to
/// read it doesn't turn the text or light a different face (decision H9).
#[derive(Clone, Copy, Debug)]
struct Frozen {
    quarters: [orientation::Quarter; smokebomb_hal::FACE_COUNT],
    up: Face,
}

/// The open menu: its draft, which way the person is holding the die, and
/// the tip being made.
struct MenuSession {
    draft: Draft,
    frame: Frame,
    tips: TipTracker,
    last_input: u64,
    turning: Option<(TipDir, f32)>,
    /// The face the turn last passed (nearest whole progress), for haptics.
    detent: i32,
}

pub struct Firmware<P: Platform>
where
    TargetOf<P>: DisplayTarget,
{
    hw: Peripherals<P>,
    sm: StateMachine,
    motion: MotionDetector,
    settings: Settings,
    /// Every app's state: the active one is the saved mode's.
    apps: Apps,
    roller: RollEngine,
    /// The smoke, or in Pig Toss the pigs (cast once a frame and laid onto
    /// every face): they share the memory.
    fx: effects::Effects,
    /// A max or dud waiting for the smoke to clear (SIM_SPEC C6).
    pending_special: Option<Special>,
    last_tick_ms: Option<u64>,
    fonts: Fonts,
    ui: Ui,
    frames: [Framebuffer<TargetOf<P>>; smokebomb_hal::FACE_COUNT],
    layer: Layer<TargetOf<P>>,
    /// A frame packed for the panel (4 bpp grey, or RGB565).
    panel: <TargetOf<P> as Target>::Panel,
    /// What each panel was last sent, for targets that send dirty tiles.
    sync: panel::PanelSync<<TargetOf<P> as DisplayTarget>::Tiles>,
    /// Filtered gravity-up direction in die coordinates (milli-g); `None`
    /// until the first IMU sample.
    up: Option<[f32; 3]>,
    orientation: TextOrientation,
    gravity: orientation::Gravity,
    /// Taps and holds from the touch mask.
    gestures: Gestures,
    /// Since when the die has been resting, if it is.
    still_since: Option<u64>,
    /// The last tap was on a die that had been resting.
    tap_deliberate: bool,
    /// The face the last tap or hold was on.
    touch_face: Face,
    /// What the hold under way will do, fixed when its ring appears (brief
    /// 3, 2.2.1): the held face shows it, and the hold does it.
    hold_pending: Option<Option<Pending>>,
    /// The menu is to open on this page alone (a game's adjust).
    open_on: Option<Page>,
    menu: Option<MenuSession>,
    up_face: Face,
    /// The screens' orientation, frozen while a result is up.
    frozen: Option<Frozen>,
    last_roll: Option<SignedRoll>,
    /// Seated in the Nest (one of its docked states).
    docked: bool,
    nest: Nest,
    reduced_motion: bool,
    /// The last time anything happened to the die, for the Sleep after
    /// setting.
    last_activity: u64,
    /// The screens are dark after sitting idle. The die still rolls if it is
    /// thrown: only the display sleeps (unlike Power off, which boots on wake).
    asleep: bool,
    /// What each face drew last frame, for the text-size audit.
    audit: [tiers::FaceAudit; smokebomb_hal::FACE_COUNT],
}

impl<P: Platform> Firmware<P>
where
    TargetOf<P>: DisplayTarget,
{
    /// Boots the firmware and returns it by value. At ~150 KB that suits the
    /// simulator and tests; on the board use [`Firmware::init`].
    pub fn new(hw: Peripherals<P>) -> HalResult<Self> {
        let mut slot = MaybeUninit::uninit();
        Self::init(&mut slot, hw)?;
        // SAFETY: `init` returned Ok, so it wrote every field.
        Ok(unsafe { slot.assume_init() })
    }

    /// Boots the firmware into `slot`, building it in place. On the board
    /// `slot` is a static: the frame buffers, smoke and fonts are far bigger
    /// than the main thread's stack, so nothing big is built on the stack
    /// and moved.
    ///
    /// On error `slot` is left uninitialised (nothing in it needs dropping).
    pub fn init(slot: &mut MaybeUninit<Self>, mut hw: Peripherals<P>) -> HalResult<&mut Self> {
        let p = slot.as_mut_ptr();
        let pack = PackIndex::load(&mut hw.assets);
        let seed = hw.rng.next_u32()?;
        // SAFETY: each field below is written once through a raw pointer and
        // none is read before `assume_init_mut`, so nothing uninitialised is
        // referenced. The pattern in `_fields` fails to compile if a field is
        // added and not listed there; add its write here too.
        unsafe {
            effects::Effects::init(
                &mut *addr_of_mut!((*p).fx).cast(),
                &mut hw.assets,
                Smoke::sprites_in(&pack),
                seed,
            );
            Fonts::init(&mut *addr_of_mut!((*p).fonts).cast(), &mut hw.assets, &pack);
        }
        let roller = RollEngine::new(&mut hw.secure_element)?;
        let settings = Settings {
            device_id: menu::short_id(&hw.secure_element.serial()?),
            ..Settings::default()
        };
        hw.display.set_enabled(true)?;
        // SAFETY: as above. The pixel buffers are plain integers, so zeroed
        // is what their `new()` gives.
        unsafe {
            addr_of_mut!((*p).frames).write_bytes(0, 1);
            addr_of_mut!((*p).layer).write_bytes(0, 1);
            addr_of_mut!((*p).panel).write_bytes(0, 1);
            addr_of_mut!((*p).sync).write(panel::PanelSync::new(TargetOf::<P>::NO_TILES));
            addr_of_mut!((*p).hw).write(hw);
            addr_of_mut!((*p).sm).write(StateMachine::new());
            addr_of_mut!((*p).motion).write(MotionDetector::new());
            addr_of_mut!((*p).settings).write(settings);
            addr_of_mut!((*p).apps).write(Apps::default());
            addr_of_mut!((*p).roller).write(roller);
            addr_of_mut!((*p).pending_special).write(None);
            addr_of_mut!((*p).last_tick_ms).write(None);
            addr_of_mut!((*p).ui).write(Ui::new());
            addr_of_mut!((*p).up).write(None);
            addr_of_mut!((*p).orientation).write(TextOrientation::new());
            addr_of_mut!((*p).gravity).write(orientation::Gravity::new());
            addr_of_mut!((*p).gestures).write(Gestures::new());
            addr_of_mut!((*p).still_since).write(None);
            addr_of_mut!((*p).tap_deliberate).write(false);
            addr_of_mut!((*p).touch_face).write(Face::PosZ);
            addr_of_mut!((*p).hold_pending).write(None);
            addr_of_mut!((*p).open_on).write(None);
            addr_of_mut!((*p).menu).write(None);
            addr_of_mut!((*p).up_face).write(Face::PosY);
            addr_of_mut!((*p).frozen).write(None);
            addr_of_mut!((*p).last_roll).write(None);
            addr_of_mut!((*p).docked).write(false);
            addr_of_mut!((*p).nest).write(Nest::new());
            addr_of_mut!((*p).reduced_motion).write(false);
            addr_of_mut!((*p).last_activity).write(0);
            addr_of_mut!((*p).asleep).write(false);
            addr_of_mut!((*p).audit).write(Default::default());
        }
        #[allow(unused_variables)]
        fn _fields<P: Platform>(f: Firmware<P>)
        where
            TargetOf<P>: DisplayTarget,
        {
            let Firmware {
                hw,
                sm,
                motion,
                settings,
                apps,
                roller,
                fx,
                pending_special,
                last_tick_ms,
                fonts,
                ui,
                frames,
                layer,
                panel,
                sync,
                up,
                orientation,
                gravity,
                gestures,
                still_since,
                tap_deliberate,
                touch_face,
                hold_pending,
                open_on,
                menu,
                up_face,
                frozen,
                last_roll,
                docked,
                nest,
                reduced_motion,
                last_activity,
                asleep,
                audit,
            } = f;
        }
        // SAFETY: every field was written above.
        Ok(unsafe { slot.assume_init_mut() })
    }

    /// The Nest: docking, guidance and its screens' state.
    pub fn nest(&self) -> &Nest {
        &self.nest
    }

    /// Reduced motion: no sinking smoke or count-up when docking, and the
    /// smoke clouds shrink.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
        if let Some(smoke) = self.fx.smoke() {
            smoke.set_reduced_motion(on);
        }
    }

    pub fn mode(&self) -> &Mode {
        self.sm.mode()
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Replaces the settings, as when they're loaded from flash at boot.
    pub fn set_settings(&mut self, settings: Settings) {
        // The die's id and licenses are its own, not settings: keep them.
        self.settings = Settings {
            device_id: self.settings.device_id,
            licensed: self.settings.licensed,
            ..settings
        };
        self.apply_settings();
    }

    /// What the die can play and is set up for, for the phone.
    pub fn inventory(&self) -> Inventory {
        Inventory {
            licensed: self.settings.licensed.with(ModeId::Dice),
            enabled: self.settings.modes(),
            active: self.settings.play().id(),
        }
    }

    /// The phone picks which licensed modes the Apps page offers. If the
    /// current mode is turned off the die goes back to Dice.
    pub fn set_enabled_modes(&mut self, enabled: ModeSet) {
        self.settings.enabled = enabled.with(ModeId::Dice);
        self.apply_settings();
    }

    /// Unlock a mode on this die. The caller has checked the store's
    /// license; verifying it here is still to do (docs/STORE.md).
    pub fn unlock_mode(&mut self, mode: ModeId) {
        self.settings.licensed = self.settings.licensed.with(mode);
        self.settings.enabled = self.settings.enabled.with(mode);
        self.apply_settings();
    }

    /// The saved mode decides whether throws roll or belong to a game.
    fn apply_settings(&mut self) {
        // Pig Toss has pigs on the faces instead of smoke, in its memory.
        if self.settings.play() == PlayMode::PigToss {
            self.fx.use_pigs();
        } else {
            let smoke = self.fx.use_smoke(&mut self.hw.assets);
            smoke.set_amount(self.settings.smoke_amount());
            smoke.set_money(self.settings.play() == PlayMode::PassThePot);
            smoke.set_reduced_motion(self.reduced_motion);
            smoke.set_up(self.gravity.up());
        }
        self.sm.set_rolls(self.app().kind().motion == MotionUse::Rolls);
        // A game in play carries on through the menu and other modes; it
        // ends only from its End game page.
        self.apps.settings_applied(&self.settings);
    }

    /// The app in use.
    fn app(&self) -> &dyn App {
        self.apps.get(self.settings.play().id())
    }

    /// Let the active app handle something, then carry out what it asks.
    fn with_app(
        &mut self,
        now: u64,
        f: impl FnOnce(&mut dyn App, &mut apps::Ctx) -> HalResult<Effects>,
    ) -> HalResult<()> {
        let id = self.settings.play().id();
        let mut cx = apps::Ctx {
            now,
            mode: *self.sm.mode(),
            label_up: self.ui.label_up(now),
            settings: &mut self.settings,
            rng: &mut self.hw.rng,
        };
        let effects = f(self.apps.get_mut(id), &mut cx)?;
        self.run_effects(effects)
    }

    /// What an app may read of the platform.
    fn view(&self, now: u64) -> apps::View<'_> {
        apps::View {
            now,
            mode: *self.sm.mode(),
            label_up: self.ui.label_up(now),
            settings: &self.settings,
        }
    }

    /// What a hold would do now: the app's pending action between throws.
    fn pending(&self, now: u64) -> Option<Pending> {
        if self.menu.is_some() || !matches!(self.sm.mode(), Mode::Idle | Mode::Reveal { .. }) {
            return None;
        }
        self.app().pending(&self.view(now))
    }

    /// Fix what the hold under way will do once its ring shows; forget it
    /// once the finger is off.
    fn latch_hold(&mut self, now: u64) {
        if !self.gestures.touching() {
            self.hold_pending = None;
        } else if self.hold_pending.is_none() && self.hold_progress(now).is_some() {
            self.hold_pending = Some(self.pending(now));
        }
    }

    /// Carry out what an app asked for.
    fn run_effects(&mut self, effects: Effects) -> HalResult<()> {
        for effect in effects {
            match effect {
                Effect::Haptic(h) => {
                    if self.settings.haptics_on() {
                        self.hw.haptics.play(h)?;
                    }
                }
                Effect::Smoke(op) => {
                    if let Some(smoke) = self.fx.smoke() {
                        match op {
                            SmokeOp::Smolder(amount) => smoke.smolder(amount),
                            SmokeOp::Burst => {
                                smoke.throw();
                                smoke.land();
                            }
                            SmokeOp::Clear => smoke.clear(),
                        }
                    }
                }
                Effect::GameStarted => self.ui.game_started(),
                Effect::DismissResult => self.ui.dismiss_result(),
            }
        }
        Ok(())
    }

    /// The Pig Toss game: scores, whose turn and the last throw.
    pub fn pigs(&self) -> &Pigs {
        &self.apps.pigs.game
    }

    /// The Hot Potato round in play, if any.
    pub fn potato(&self) -> &Potato {
        &self.apps.potato
    }

    pub fn last_roll(&self) -> Option<&SignedRoll> {
        self.last_roll.as_ref()
    }

    pub fn frames(&self) -> &[Framebuffer<TargetOf<P>>; smokebomb_hal::FACE_COUNT] {
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

    /// The particle system, for tests that replay the mockup's random
    /// sequence. None in Pig Toss, which has the pigs instead.
    pub fn smoke_mut(&mut self) -> Option<&mut Smoke> {
        self.fx.smoke()
    }

    /// The text orientation the menu page on `face` is drawn in, if one is.
    pub fn menu_page_quarter(&self, face: Face) -> Option<orientation::Quarter> {
        let frame = page_frame(self.menu.as_ref()?, face)?;
        Some(page_quarter(&frame, face, &self.orientation))
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
            // Smoke falls with gravity even while the die is shaken or
            // tumbling, as in the mockup.
            self.gravity.update(&sample, IMU_SAMPLE_S);
            let up = self.gravity.up();
            if let Some(smoke) = self.fx.smoke() {
                smoke.set_up(up);
            }
            if let Some(face) = motion::up_face(&sample) {
                self.up_face = face;
            }
            if let Some(m) = self.motion.update(&sample, now) {
                let _ = events.push(Event::Motion(m));
            }
            self.menu_tips(&sample, IMU_SAMPLE_S, now)?;
        }
        if let Some(m) = &self.menu {
            if m.turning.is_none() && now.saturating_sub(m.last_input) >= MENU_IDLE_MS {
                let _ = events.push(Event::MenuTimeout);
            }
        }

        if self.motion.is_still() {
            self.still_since.get_or_insert(now);
        } else {
            self.still_since = None;
        }
        self.latch_hold(now);
        if self.poll_touch(now, &mut events)? {
            self.ui.touch(now);
        }

        let battery = self.hw.power.battery()?;
        self.sense_nest(now, &battery, &mut events)?;
        self.ui
            .set_low_battery(battery.percent <= LOW_BATTERY_PCT && !self.docked);

        // BLE: drain writes from the phone. Decoding into `PhoneToDie` and
        // acting on it is not wired up yet.
        let mut rx = [0u8; 64];
        while self.hw.ble.receive(&mut rx)?.is_some() {}

        let before = *self.sm.mode();
        let input = !events.is_empty();
        let _ = events.push(Event::Tick);
        for event in events {
            let outside = !matches!(self.sm.mode(), Mode::Menu | Mode::Nest | Mode::Off);
            match event {
                Event::Motion(Motion::Shaking) if outside && self.app().kind().motion == MotionUse::Shake => {
                    self.with_app(now, |app, cx| app.shaken(cx))?;
                }
                // No opening the menu mid-round.
                Event::LongPress if outside && self.app().blocks_hold() => continue,
                // A hold does what the held screen shows: the game's pending
                // action, or the menu (brief 3, 2.4).
                Event::LongPress => {
                    let pending = match self.hold_pending.take() {
                        Some(latched) => latched,
                        None => self.pending(now),
                    };
                    match pending.map(|p| p.action) {
                        Some(Action::Open(page)) => self.open_on = Some(page),
                        Some(action) => {
                            self.with_app(now, |app, cx| Ok(app.commit(action, cx)))?;
                            if self.settings.haptics_on() {
                                self.hw.haptics.play(smokebomb_hal::HapticEffect::MenuSave)?;
                            }
                            self.ui.committed(now);
                            continue;
                        }
                        None => {}
                    }
                }
                // Taps are read-only: the app may change what the screens
                // show, and the face says what a hold would do.
                Event::Tap => {
                    let tap = Tap {
                        face: self.touch_face,
                        up: self.up_face,
                        deliberate: self.tap_deliberate,
                    };
                    let effects = self.app().tapped(tap, &self.view(now));
                    self.run_effects(effects)?;
                    self.ui.tap(now, self.sm.mode());
                    if self.tap_deliberate {
                        self.ui.label_tapped(now);
                    }
                    if let Some(p) = self.pending(now) {
                        self.ui.hint(now, self.touch_face, p.hint);
                    }
                }
                Event::Docked(true) => {
                    let effects = self.apps.docked();
                    self.run_effects(effects)?;
                }
                _ => {}
            }
            let commands = self.sm.handle(event, now);
            for cmd in commands {
                self.execute(cmd, now)?;
            }
        }
        self.with_app(now, |app, cx| Ok(app.tick(cx)))?;
        let after = *self.sm.mode();
        self.update_sleep(now, input, &after)?;
        self.ui.tick(now, &before, &after, self.up_face, self.docked);
        self.update_smoke(now, &before, &after);
        self.freeze_for_result(&before, &after);

        self.render(now)?;
        Ok(())
    }

    /// Read the Nest's magnet when it's time, and let the dock state machine
    /// decide whether the die is seated, wrongly seated, unpowered or lifted.
    fn sense_nest(
        &mut self,
        now: u64,
        battery: &smokebomb_hal::BatteryStatus,
        events: &mut Vec<Event, 8>,
    ) -> HalResult<()> {
        let still = self.motion.is_still();
        if self.nest.wants_reading(now, still, self.asleep) {
            // The display's 12 V supply is paused for the reading so its
            // current can't disturb the measurement.
            self.hw.display.set_supply_paused(true)?;
            let raw = self.hw.mag.read();
            self.hw.display.set_supply_paused(false)?;
            let (raw, offset) = (raw?, self.hw.mag.hard_iron());
            let b = [0, 1, 2].map(|i| raw[i] - offset[i]);
            self.nest.reading(now, b, self.up.unwrap_or([0.0, 1000.0, 0.0]));
        }
        let step = self.nest.update(nest::Inputs {
            now,
            still,
            up_face: self.up_face,
            vbus: battery.vbus,
            charge: battery.charge,
        });
        self.docked = self.nest.docked();
        match step.change {
            nest::Change::None => {}
            nest::Change::Docked => {
                let _ = events.push(Event::Docked(true));
            }
            nest::Change::Undocked { animate } => {
                let _ = events.push(Event::Docked(false));
                if animate && !matches!(self.sm.mode(), Mode::Menu) {
                    // The battery holds on the top face, then the setup label.
                    self.ui
                        .wake_from(now + nest::UNDOCK_HOLD_MS, ui::WAKE_AFTER_BOOT_MS);
                }
            }
        }
        if let Some(effect) = step.haptic {
            if self.settings.haptics_on() {
                self.hw.haptics.play(effect)?;
            }
        }
        Ok(())
    }

    /// Seconds since the winning throw's score gave way to the win screen,
    /// once it has.
    fn pig_win_t(&self, now: u64) -> Option<f32> {
        if self.settings.play() != PlayMode::PigToss {
            return None;
        }
        self.apps.pigs.win_t(now, self.sm.mode())
    }

    /// The Sleep after setting: the screens go dark once the die has sat
    /// idle that long, and light again (with the setup label, no boot) when
    /// it is touched, picked up or thrown.
    fn update_sleep(&mut self, now: u64, input: bool, mode: &Mode) -> HalResult<()> {
        let busy = input
            || self.gestures.touching()
            || !matches!(mode, Mode::Idle)
            || self.app().busy()
            || self.ui.booting();
        if busy {
            self.last_activity = now;
            if self.asleep {
                self.asleep = false;
                self.hw.display.set_enabled(true)?;
                self.ui.woke(now);
            }
        } else if !self.asleep {
            if let Some(ms) = self.settings.sleep_after_ms() {
                if now.saturating_sub(self.last_activity) >= ms {
                    self.asleep = true;
                    self.hw.display.set_enabled(false)?;
                }
            }
        }
        Ok(())
    }

    /// Freeze the screens' orientation when a result is revealed, and let
    /// it go once the result is cleared.
    fn freeze_for_result(&mut self, before: &Mode, after: &Mode) {
        let revealed = matches!(after, Mode::Reveal { .. }) && !matches!(before, Mode::Reveal { .. });
        if revealed {
            self.frozen = Some(Frozen {
                quarters: Face::ALL.map(|f| self.orientation.quarter(f)),
                up: self.up_face,
            });
        } else if !self.ui.has_result() {
            self.frozen = None;
        }
    }

    /// Which face is up, as the screens see it (frozen while a result is up).
    pub fn display_up(&self) -> Face {
        self.frozen.map_or(self.up_face, |f| f.up)
    }

    /// A face's text orientation, as drawn (frozen while a result is up).
    pub fn display_quarter(&self, face: Face) -> orientation::Quarter {
        self.frozen
            .map_or_else(|| self.orientation.quarter(face), |f| f.quarters[face.index()])
    }

    /// Drive the smoke through the throw, in the mockup's frame order:
    /// throw and landing first, then a step.
    fn update_smoke(&mut self, now: u64, before: &Mode, after: &Mode) {
        let entered = |m: fn(&Mode) -> bool| !m(before) && m(after);
        if entered(|m| matches!(m, Mode::Shaking) || matches!(m, Mode::Airborne)) {
            self.pending_special = None;
        }
        let dt = self.frame_dt(now);
        self.apps.pigs.animate(dt, before, after, now);
        // Pig Toss has no smoke.
        let Some(smoke) = self.fx.smoke() else {
            return;
        };
        if entered(|m| matches!(m, Mode::Shaking)) {
            smoke.shake_start();
        }
        if entered(|m| matches!(m, Mode::Airborne)) {
            smoke.throw();
        }
        // Landed: the die has stopped after the tumble (the mockup's
        // landing, about 0.35 s before the result shows). Shaken and set
        // straight down counts as a throw that has landed.
        let resting = self.motion.stopped() || matches!(after, Mode::Reveal { .. });
        if smoke.shaking() && matches!(after, Mode::Reveal { .. }) {
            smoke.throw();
        }
        if smoke.tumbling() && resting && !matches!(after, Mode::Airborne) {
            smoke.land();
        }
        if entered(|m| matches!(m, Mode::Menu) || matches!(m, Mode::Nest)) {
            smoke.clear();
        }
        smoke.step(dt);
        // A max or dud shows once the smoke has cleared from the result.
        if let Some(special) = self.pending_special {
            if self.ui.showing_result(now) && !smoke.has_smoke() && !smoke.tumbling() {
                smoke.special(special);
                self.ui.set_special(now, special);
                self.pending_special = None;
            }
        }
    }

    /// Seconds since the last tick, for animation: exactly one tick period
    /// when the tick came on time (a millisecond clock would otherwise make
    /// steps of 16 and 17 ms), the measured gap when it didn't.
    fn frame_dt(&mut self, now: u64) -> f32 {
        let elapsed = self.last_tick_ms.map_or(0, |last| now.saturating_sub(last));
        self.last_tick_ms = Some(now);
        let nominal = 1000.0 / TICK_HZ as f32;
        match elapsed {
            0 => 0.0,
            e if (e as f32 - nominal).abs() <= 2.0 => 1.0 / TICK_HZ as f32,
            e => e as f32 / 1000.0,
        }
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
            m.frame = Frame::new(m.frame.front_face(), up, m.frame.up);
        }
        match update {
            TipUpdate::Turning { dir, progress } => {
                // A tick each time the nearest face changes (45° into each
                // face passed): once for a quick tip, as the mockup's buzz
                // at its start, and once per face on a long turn, however
                // many stops it makes on the way.
                let detent = libm::floorf(progress + 0.5) as i32;
                if detent != m.detent && self.settings.haptics_on() {
                    self.hw.haptics.play(smokebomb_hal::HapticEffect::MenuTip)?;
                }
                m.detent = detent;
                m.turning = Some((dir, progress));
                m.last_input = now;
            }
            TipUpdate::Done { dir, steps } => {
                m.draft = m.draft.stepped(dir, steps);
                m.frame = m.frame.stepped(dir, steps);
                m.turning = None;
                m.detent = 0;
                m.last_input = now;
            }
            TipUpdate::Cancelled => m.turning = None,
            TipUpdate::None => {}
        }
        if matches!(update, TipUpdate::Done { .. } | TipUpdate::Cancelled) {
            // At rest: square the frame to gravity. That catches what the
            // gyro can't be trusted with over time (wobble, tilts, drift) and
            // what isn't a tip (a twist): a tilt onto the next face counts as
            // the tip it amounts to, a roll just turns the page upright.
            let (frame, steps, rolled) = m.frame.resync(self.gravity.up());
            if steps != 0 {
                m.draft = m.draft.stepped(TipDir::Up, steps);
            }
            if frame != m.frame {
                m.frame = frame;
                m.tips.resynced(rolled);
            }
        }
        Ok(())
    }

    /// Returns true when a touch starts.
    fn poll_touch(&mut self, now: u64, events: &mut Vec<Event, 8>) -> HalResult<bool> {
        let mask = self.hw.touch.read()?;
        // Touches while the die is moving don't count. In the menu the die
        // is in the hand anyway; only a tip in progress blocks a hold.
        let steady = match &self.menu {
            Some(m) => m.turning.is_none(),
            None => self.motion.is_still(),
        };
        let input = gesture::Input {
            now,
            mask,
            steady,
            rested_ms: self.still_since.map_or(0, |s| now.saturating_sub(s)),
        };
        let Some(gesture) = self.gestures.update(input) else {
            return Ok(false);
        };
        match gesture {
            Gesture::Touched => {
                self.nest.touch(now);
                return Ok(true);
            }
            Gesture::Hold { face } => {
                self.touch_face = face;
                // In the menu a hold saves, unless the draft has another
                // step first or it's on Power.
                let event = match self.menu.as_ref().map(|m| m.draft.held()) {
                    Some(Held::Next(_)) => Event::MenuNext,
                    Some(Held::PowerOff) => Event::PowerOff,
                    _ => Event::LongPress,
                };
                let _ = events.push(event);
            }
            // Letting go after a hold that saved the menu shows the setup
            // like a tap does (the mockup's pointer-up); letting go after the
            // hold that opened it doesn't.
            Gesture::Released { face } => {
                // Not after the hold that powered the die off: that would
                // wake it again.
                if self.menu.is_none() && !matches!(self.sm.mode(), Mode::Off) {
                    self.touch_face = face;
                    self.tap_deliberate = false;
                    let _ = events.push(Event::Tap);
                }
            }
            Gesture::Tap { face, deliberate } => {
                self.touch_face = face;
                self.tap_deliberate = deliberate;
                let _ = events.push(Event::Tap);
            }
        }
        Ok(false)
    }

    /// The face being held toward a hold and how far it has got (0–1),
    /// while the ring shows.
    fn hold_progress(&self, now: u64) -> Option<(Face, f32)> {
        let ring = !self.app().blocks_hold()
            && matches!(
                self.sm.mode(),
                Mode::Idle | Mode::Reveal { .. } | Mode::Menu | Mode::Nest
            );
        self.gestures.hold_progress(now).filter(|_| ring)
    }

    /// Carry out a command. `now` is the tick's time: the clock moves on
    /// while a tick runs, and everything in one tick must agree on when it is.
    fn execute(&mut self, cmd: Command, now: u64) -> HalResult<()> {
        match cmd {
            Command::Haptic(effect) => {
                if self.settings.haptics_on() {
                    self.hw.haptics.play(effect)?;
                }
            }
            Command::Roll => {
                let Throw::Dice(die, count) = self.app().throw(&self.settings) else {
                    // The app's own throw: nothing is signed.
                    self.pending_special = None;
                    return self.with_app(now, |app, cx| app.landed(cx));
                };
                let signed =
                    self.roller
                        .roll(&mut self.hw.rng, &mut self.hw.secure_element, die, count, now)?;
                self.pending_special = special(&signed, die);
                if self.pending_special == Some(Special::Max) && self.settings.haptics_on() {
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
                self.apps.menu_opened();
                let up = self.up.unwrap_or([0.0, 1.0, 0.0]);
                let live = self.apps.pigs.live;
                let draft = match self.open_on.take() {
                    Some(page) => Draft::alone(&self.settings, page),
                    None => Draft::new(&self.settings),
                };
                self.menu = Some(MenuSession {
                    draft: draft.with_session(live),
                    frame: Frame::new(
                        self.touch_face,
                        up,
                        orientation::sky_for(self.touch_face, self.display_quarter(self.touch_face)),
                    ),
                    // Starts disarmed: tips count once the die is still.
                    tips: TipTracker::new(),
                    last_input: now,
                    turning: None,
                    detent: 0,
                });
                self.ui.menu_opened(now);
            }
            Command::MenuNext => {
                if let Some(m) = &mut self.menu {
                    if let Held::Next(next) = m.draft.held() {
                        m.draft = next;
                        m.last_input = now;
                        // The new page grows in as the menu did opening.
                        self.ui.menu_opened(now);
                    }
                }
            }
            // The draft is dropped: powering off saves nothing.
            Command::MenuPowerOff => {
                if self.menu.take().is_some() {
                    if let Some(smoke) = self.fx.smoke() {
                        smoke.clear();
                    }
                    self.pending_special = None;
                    self.ui.power_off();
                    self.hw.display.set_enabled(false)?;
                }
            }
            Command::WakeUp => {
                self.hw.display.set_enabled(true)?;
                self.ui.wake_up(now);
            }
            Command::MenuClose { save } => {
                if let Some(m) = self.menu.take() {
                    if save {
                        // Ending a game, or setting up a new one, happens
                        // only now: backing out of the menu leaves it be.
                        if m.draft.ended {
                            self.apps.pigs.end();
                        }
                        if m.draft.set_up() || m.draft.restart() {
                            self.apps.pigs.start(m.draft.players);
                        }
                        m.draft.commit(&mut self.settings);
                        self.apply_settings();
                        self.apply_hardware_settings()?;
                    }
                    self.ui.menu_closed(now, m.frame.front_face(), m.draft, save);
                }
            }
        }
        Ok(())
    }

    /// Push saved settings to the hardware they control. Smoke and the mode
    /// are applied by `apply_settings`, and Sleep after by `update_sleep`.
    /// Owner and About are display-only.
    fn apply_hardware_settings(&mut self) -> HalResult<()> {
        let level = (self.settings.brightness_pct() as u16 * 255 / 100) as u8;
        for face in Face::ALL {
            self.hw.display.set_brightness(face, level)?;
        }
        self.hw.ble.set_advertising(self.settings.bluetooth_on())
    }

    /// What each face shows of the Nest: its docked screens, or the
    /// battery and clock hanging on for a moment after the die is lifted.
    fn nest_faces(
        &mut self,
        now: u64,
        mode: &Mode,
        battery: u8,
    ) -> [Option<NestFace>; smokebomb_hal::FACE_COUNT] {
        let docked = matches!(mode, Mode::Nest);
        if !docked && (self.ui.blackout() || matches!(mode, Mode::Menu | Mode::Off) || !self.nest.undocking())
        {
            return [None; smokebomb_hal::FACE_COUNT];
        }
        let charge = self
            .hw
            .power
            .battery()
            .map_or(smokebomb_hal::ChargeState::Idle, |b| b.charge);
        let secs = self.nest.clock_time(now, self.hw.clock.local_seconds());
        let quarters = Face::ALL.map(|f| self.display_quarter(f));
        let env = nest::Env {
            battery: if charge == smokebomb_hal::ChargeState::Full {
                100
            } else {
                battery
            },
            charge,
            secs,
            night: self.settings.night_hours,
            reduced_motion: self.reduced_motion,
            quarters: &quarters,
        };
        Face::ALL.map(|f| {
            if docked {
                Some(self.nest.face(now, f, &env))
            } else {
                self.nest.undock_face(now, f, &env)
            }
        })
    }

    fn render(&mut self, now: u64) -> HalResult<()> {
        let mode = *self.sm.mode();
        let up = self.display_up();
        let quarters = Face::ALL.map(|f| self.display_quarter(f));
        let setup = self.settings.setup();
        let pigs_on = self.settings.play() == PlayMode::PigToss;
        let label = setup.label();
        // Between throws the Pig Toss label says whose go it is, or who won.
        let tokens = self.settings.tokens;
        let pigs_up = self
            .apps
            .pigs
            .game
            .winner()
            .unwrap_or(self.apps.pigs.game.current());
        let pigs_won = self.apps.pigs.game.winner().is_some();
        let battery = self.hw.power.battery()?.percent;
        let potato_view = if matches!(mode, Mode::Menu | Mode::Nest) {
            None
        } else if let Some(t) = self.apps.potato.boomed_for(now) {
            Some(PotatoView::Boom(t as f32 / 1000.0))
        } else if self.apps.potato.is_lit() {
            Some(PotatoView::Fuse(
                self.apps.potato.heat(now),
                self.apps.potato.pulse(now),
                (now % 1_000_000) as f32 / 1000.0,
            ))
        } else {
            None
        };
        // Pig Toss: the pigs tumble while the die is in the air, and settle
        // into their poses once it lands, for as long as the result is up.
        let since_landing = now.saturating_sub(self.apps.pigs.landed_ms) as f32 / 1000.0;
        let players = self.apps.pigs.game.players();
        // A winning throw's score gives way to the win screen.
        let pig_win = self.pig_win_t(now).and_then(|t| {
            let fade = (LOCKED_MS as f32 / 1000.0 - t).clamp(0.0, 1.5) / 1.5;
            let winner = self.apps.pigs.game.winner()?;
            (fade > 0.0).then(|| {
                (
                    tokens[winner as usize],
                    self.apps.pigs.game.scores()[winner as usize],
                    t,
                    fade,
                )
            })
        });
        let win_up = pig_win.is_some();
        let pig_scene = if !pigs_on || win_up || matches!(mode, Mode::Menu | Mode::Nest | Mode::Off) {
            None
        } else if matches!(mode, Mode::Shaking | Mode::Airborne | Mode::Settling) {
            Some(pigfx::Scene::Tumbling(self.apps.pigs.clock))
        } else {
            self.apps
                .pigs
                .game
                .last()
                .filter(|_| self.ui.showing_result(now))
                .map(|t| pigfx::Scene::Settling {
                    land: self.apps.pigs.land,
                    u: since_landing / pigfx::SETTLE_S,
                    shrink: TargetOf::<P>::pig_settle(since_landing).0,
                    poses: t.poses,
                    touching: t.touching,
                })
        };
        let pig_alpha = if matches!(mode, Mode::Shaking | Mode::Airborne | Mode::Settling) {
            1.0
        } else {
            self.ui.result_dim(now)
                * screens::pig_win::score_fade(self.apps.pigs.game.last(), since_landing)
                * TargetOf::<P>::pig_settle(since_landing).1
        };
        // A bank locking in: a padlock snaps shut and the score counts up.
        let locked = self
            .apps
            .pigs
            .locked
            .filter(|_| pigs_on && pig_scene.is_none() && matches!(mode, Mode::Idle | Mode::Reveal { .. }))
            .filter(|(_, at)| now.saturating_sub(*at) < LOCKED_MS)
            .map(|(l, at)| (l, now.saturating_sub(at) as f32 / 1000.0));
        let hold = self.hold_progress(now);
        // While the ring fills, the held face shows what the hold will do.
        let preview = hold.and_then(|(f, _)| Some((f, self.hold_pending.clone()??)));
        let asleep = self.asleep;
        let nest_faces = self.nest_faces(now, &mode, battery);
        let Self {
            frames,
            layer,
            fonts,
            hw,
            ui,
            orientation,
            last_roll,
            menu,
            apps,
            fx,
            audit,
            ..
        } = self;
        let record = last_roll.as_ref().map(|r| &r.record);
        let pigs_throw = if pigs_on {
            apps.pigs.game.last().copied()
        } else {
            None
        };
        let has_result = if pigs_on {
            pigs_throw.is_some()
        } else {
            record.is_some()
        };
        let blackout = ui.blackout() || asleep;
        // Every face shows the same pigs: cast them once for all of them.
        if let Some(scene) = pig_scene.filter(|_| !blackout) {
            if let Some(pigs) = fx.pigs() {
                pigs.draw_through(scene, TargetOf::<P>::pig_lens());
            }
        }

        for face in Face::ALL {
            let fb = &mut frames[face.index()];
            fb.clear();
            audit[face.index()] = tiers::FaceAudit::default();
            if blackout {
                continue;
            }
            let mut content = ui.content(now, face, up, &mode, has_result);
            // Lifted from the Nest a moment ago: its screens hang on.
            if nest_faces[face.index()].is_some() && !matches!(content, FaceContent::Menu) {
                content = FaceContent::Nest;
            }
            // A round of Hot Potato takes the faces, except the one facing
            // down (H2).
            let potato_face = potato_view.filter(|_| face != up.opposite());
            let previewing = preview.as_ref().filter(|(f, _)| *f == face && menu.is_none());
            let down = face == up.opposite();
            let screen = if previewing.is_some()
                || matches!(content, FaceContent::Menu | FaceContent::Success { .. })
            {
                tiers::Screen::Held
            } else if potato_face.is_some()
                || (!down && (locked.is_some() || win_up))
                || matches!(
                    content,
                    FaceContent::Result { .. } | FaceContent::Wake { .. } | FaceContent::LowBattery { .. }
                )
            {
                tiers::Screen::Table
            } else {
                tiers::Screen::Other
            };
            let content = if potato_face.is_some() || locked.is_some() || win_up || previewing.is_some() {
                FaceContent::Blank
            } else {
                content
            };
            let rot = quarters[face.index()];
            let mut painter = Painter::new(fb, layer, Transform::quarter_on::<TargetOf<P>>(rot));
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
                TargetOf::<P>::draw_menu(
                    &mut c,
                    &draft,
                    battery as f32 / 100.0,
                    0.0,
                    0.0,
                    1.0 - u,
                    1.0 + 0.12 * u,
                );
                TargetOf::<P>::draw_hold_ring(&mut c, 1.0, 1.0 - u, u * 3.0);
            }
            if menu.is_none() {
                if let Some((f, p)) = hold {
                    if f == face {
                        TargetOf::<P>::draw_hold_ring(&mut c, p, 1.0, 0.0);
                    }
                }
            }

            // A tap's hint on this face: its content moves down to clear it.
            let hint = ui.hint_on(now, face).filter(|_| previewing.is_none());
            let base = c.painter.xf;
            if hint.is_some() {
                c.painter.xf = base.offset(0.0, TargetOf::<P>::HINT_ROOM);
            }
            match content {
                FaceContent::Blank => {}
                FaceContent::Boot { t, top } => TargetOf::<P>::draw_boot(&mut c, face.index(), top, t),
                FaceContent::Wake { alpha } if pigs_on => {
                    TargetOf::<P>::draw_pigs_label(&mut c, setup, tokens[pigs_up as usize], pigs_won, alpha)
                }
                FaceContent::Wake { alpha } => {
                    TargetOf::<P>::draw_wake_label(&mut c, setup, &label, alpha, face.index())
                }
                FaceContent::Result { alpha } => {
                    if let Some(t) = pigs_throw {
                        let next = (t.player + 1) % players;
                        let alpha = alpha * screens::pig_win::score_fade(Some(&t), since_landing);
                        TargetOf::<P>::draw_pig_score(&mut c, &t, since_landing, next, &tokens, alpha);
                    } else if let Some(r) = record {
                        TargetOf::<P>::draw_result(&mut c, r, ui.special(), alpha);
                    }
                }
                FaceContent::Success { t, setup } => {
                    TargetOf::<P>::draw_success(&mut c, &setup.short_label(), setup.nudge(), t);
                }
                FaceContent::Menu => {
                    if let Some(m) = menu {
                        draw_menu_face(
                            &mut c,
                            m,
                            ui,
                            orientation,
                            face,
                            now,
                            battery,
                            hold.map(|(_, p)| p),
                        );
                    }
                }
                FaceContent::Nest => {
                    if let Some(nf) = nest_faces[face.index()].filter(|nf| nf.dim > 0.0) {
                        TargetOf::<P>::draw_nest(&mut c, &nf);
                        fb_of(c.painter).scale(nf.dim);
                    }
                }
                FaceContent::LowBattery { alpha } => TargetOf::<P>::draw_bolt(&mut c, alpha),
            }
            if let Some((l, t)) = locked {
                if face != up.opposite() {
                    TargetOf::<P>::draw_locked(
                        &mut c,
                        &l,
                        tokens[l.next as usize],
                        t,
                        (LOCKED_MS as f32 / 1000.0 - t).clamp(0.0, 1.5) / 1.5,
                    );
                }
            }
            if let Some((player, total, t, fade)) = pig_win {
                if face != up.opposite() {
                    TargetOf::<P>::draw_pig_win(&mut c, player, total, t, fade);
                }
            }
            if let Some(scene) = pig_scene {
                // In the air every face shows the pigs: which one faces down
                // changes all the time, and would blink on and off. Landed,
                // the face-down screen stays dark (H2).
                let in_air = matches!(scene, pigfx::Scene::Tumbling(_));
                if in_air || face != up.opposite() {
                    if let Some(pigs) = fx.pigs() {
                        pigs.lay_onto_tinted(fb_of(c.painter), rot, pig_alpha, TargetOf::<P>::pig_tint());
                    }
                }
            }
            match potato_face {
                Some(PotatoView::Fuse(heat, pulse, t)) => TargetOf::<P>::draw_fuse(&mut c, heat, pulse, t),
                Some(PotatoView::Boom(t)) => TargetOf::<P>::draw_boom(&mut c, t),
                None => {}
            }
            c.painter.xf = base;
            if let Some((_, p)) = previewing {
                TargetOf::<P>::draw_hold_preview(&mut c, p.word, &p.value);
            } else if let Some((text, alpha)) = hint {
                c.painter.mark_kind = gfx::MarkKind::Hint;
                TargetOf::<P>::draw_tap_hint(&mut c, text, alpha);
                c.painter.mark_kind = gfx::MarkKind::Text;
            }
            audit[face.index()] = tiers::FaceAudit {
                screen,
                marks: core::mem::take(&mut c.painter.marks),
            };
        }

        // Smoke over everything, wrapping round the edges (SIM_SPEC D1).
        if let Some(smoke) = self.fx.smoke_ref() {
            smoke.draw(&mut self.frames);
        }
        match TargetOf::<P>::DELIVERY {
            Delivery::WholeFrames => {
                for face in Face::ALL {
                    TargetOf::<P>::pack(&self.frames[face.index()], &mut self.panel);
                    self.hw.display.write_frame(face, &self.panel)?;
                }
            }
            Delivery::Dirty {
                bytes_per_tick,
                face_down_ms,
            } => self.deliver_dirty(now, up.opposite(), bytes_per_tick, face_down_ms)?,
        }
        self.hw.display.flush()
    }

    /// Send what changed, within the bus budget ([`panel`]).
    fn deliver_dirty(&mut self, now: u64, down: Face, budget: usize, down_every_ms: u64) -> HalResult<()> {
        let layout = TargetOf::<P>::LAYOUT;
        let mut hashes = [TargetOf::<P>::NO_TILES; smokebomb_hal::FACE_COUNT];
        for face in Face::ALL {
            TargetOf::<P>::pack(&self.frames[face.index()], &mut self.panel);
            panel::hash_tiles(self.panel.as_ref(), layout, hashes[face.index()].as_mut());
        }
        let plan = self.sync.plan(&hashes, layout, now, down, down_every_ms, budget);
        for face in Face::ALL {
            let (i, region) = (face.index(), plan[face.index()]);
            if region.is_empty() {
                continue;
            }
            TargetOf::<P>::pack(&self.frames[i], &mut self.panel);
            self.hw.display.write_region(face, region, &self.panel)?;
            self.sync.sent(i, region, &hashes[i], layout, now);
        }
        Ok(())
    }

    /// What each face drew last frame: its kind of screen and every text on
    /// it, for the text-size audit (brief 3, test 1).
    pub fn text_audit(&self) -> &[tiers::FaceAudit; smokebomb_hal::FACE_COUNT] {
        &self.audit
    }

    /// Bytes sent to each panel so far by dirty-tile delivery (zero on a
    /// target that sends whole frames).
    pub fn panel_bytes(&self) -> [u64; smokebomb_hal::FACE_COUNT] {
        self.sync.bytes
    }
}

/// What a round of Hot Potato shows on the faces.
#[derive(Clone, Copy)]
enum PotatoView {
    /// Heat and pulse, both 0–1, and the clock in seconds (wrapping every
    /// ~17 minutes, so it stays precise as an f32), for animation.
    Fuse(f32, f32, f32),
    /// Seconds since it went off.
    Boom(f32),
}

/// One face while the menu is open (C3). The menu shows on the front face;
/// during a tip the old page slides off against the turn and fades while
/// the new one slides in from the leading edge of the face coming round. On
/// a turn past several faces the slide runs between the two faces the die
/// is between, each showing the page it stands for.
/// Each page is drawn upright for the frame it belongs to, the way it will
/// read once its face is in front: a face coming round from the top or
/// bottom would otherwise keep the orientation it had there until it
/// counted as a side face, and flip mid-turn.
#[allow(clippy::too_many_arguments)]
fn draw_menu_face<A: smokebomb_hal::AssetStore, T: DisplayTarget>(
    c: &mut Ctx<A, T>,
    m: &MenuSession,
    ui: &Ui,
    orientation: &TextOrientation,
    face: Face,
    now: u64,
    battery: u8,
    hold: Option<f32>,
) {
    let battery = battery as f32 / 100.0;
    let Some(page) = page_frame(m, face) else {
        return;
    };
    c.painter.xf = Transform::quarter_on::<T>(page_quarter(&page, face, orientation));
    if let Some((dir, progress)) = m.turning {
        // `k` whole faces passed, and `u` of the way to the next.
        let k = libm::floorf(progress);
        let u = progress - k;
        let (frame, draft) = (m.frame.stepped(dir, k as i32), m.draft.stepped(dir, k as i32));
        let (mx, my) = frame.motion_dir(face, dir);
        let d = screens::TIP_SLIDE;
        if face == frame.front_face() {
            T::draw_menu(c, &draft, battery, -mx * u * d, -my * u * d, 1.0 - u, 1.0);
        } else {
            let next = draft.tipped(dir);
            let (ox, oy) = (mx * (1.0 - u) * d, my * (1.0 - u) * d);
            T::draw_menu(c, &next, battery, ox, oy, u, 1.0);
        }
        return;
    }
    {
        let intro = ui.menu_intro(now);
        T::draw_hold_ring(c, 1.0, intro.ring_alpha, intro.ring_grow);
        T::draw_menu(c, &m.draft, battery, 0.0, 0.0, intro.alpha, intro.scale);
        if let Some(p) = hold {
            T::draw_hold_ring(c, p, 1.0, 0.0);
        }
    }
}

/// The menu frame whose page `face` shows: the front face's, or during a
/// turn, the page being left or the one coming round.
fn page_frame(m: &MenuSession, face: Face) -> Option<Frame> {
    match m.turning {
        Some((dir, progress)) => {
            let frame = m.frame.stepped(dir, libm::floorf(progress) as i32);
            if face == frame.front_face() {
                Some(frame)
            } else if face == frame.next_front(dir) {
                Some(frame.after(dir))
            } else {
                None
            }
        }
        None => (face == m.frame.front_face()).then_some(m.frame),
    }
}

/// Upright for `frame`'s sky; the live orientation if that's undefined
/// (the held face pointing up, SIM_SPEC H7).
fn page_quarter(frame: &Frame, face: Face, live: &TextOrientation) -> orientation::Quarter {
    orientation::upright(face, frame.up).unwrap_or_else(|| live.quarter(face))
}

fn fb_of<'a, T: Target>(painter: &'a mut Painter<'_, T>) -> &'a mut Framebuffer<T> {
    painter.framebuffer()
}

/// A max (every die shows its top value, N > 2) or a dud (every die shows
/// 1). Pass the Pot has neither (SIM_SPEC C6).
fn special(roll: &SignedRoll, die: DieKind) -> Option<Special> {
    let values = &roll.record.values;
    if !die.is_numeric() || values.is_empty() {
        None
    } else if die.sides() > 2 && values.iter().all(|&v| v == die.sides()) {
        Some(Special::Max)
    } else if values.iter().all(|&v| v == 1) {
        Some(Special::Dud)
    } else {
        None
    }
}
