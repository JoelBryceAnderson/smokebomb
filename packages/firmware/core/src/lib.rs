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

pub mod bench;
pub mod display;
pub mod effects;
pub mod font;
pub mod gfx;
pub mod icons;
pub mod menu;
pub mod motion;
pub mod nest;
pub mod orientation;
pub mod pack;
pub mod pigfx;
pub mod pigs;
pub mod potato;
pub mod roll;
pub mod screens;
pub mod session;
pub mod smoke;
pub mod state;
pub mod target;
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
use smokebomb_shared::types::MAX_POT_DICE;
use smokebomb_shared::{DieKind, ModeId, ModeSet, SignedRoll};

use display::Framebuffer;
use font::Fonts;
use gfx::{Layer, Painter, Transform};
use menu::{Draft, Held, PlayMode, Settings};
use motion::{Motion, MotionDetector};
use nest::{Nest, NestFace};
use orientation::TextOrientation;
use pack::PackIndex;
use pigs::Pigs;
use potato::{Potato, PotatoCommand};
use roll::RollEngine;
use screens::Ctx;
use session::Sessions;
use smoke::{Smoke, Special};
use state::{Command, Event, Mode, StateMachine};
use target::DisplayTarget;
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

/// How long a screen must be held to open the menu (SIM_SPEC C3).
pub const MENU_HOLD_MS: u64 = 800;
/// A tap that banks in Pig Toss must be short, and land on a die that has
/// been resting: picking the die up puts a finger on the top screen too.
pub const TAP_MAX_MS: u64 = 500;
pub const TAP_REST_MS: u64 = 400;
/// How long the lock-in stays up before the screens go quiet.
pub const LOCKED_MS: u64 = 12_000;
/// The hold ring appears once a touch is clearly a hold, not a tap.
pub const HOLD_RING_AFTER_MS: u64 = 220;
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

pub struct Firmware<P: Platform> {
    hw: Peripherals<P>,
    sm: StateMachine,
    motion: MotionDetector,
    settings: Settings,
    potato: Potato,
    sessions: Sessions,
    /// Seconds into the pigs' tumble, and how far in they were when the
    /// die landed. Runs only while the die is shaken or thrown.
    pig_clock: f32,
    pig_land: f32,
    /// When the die landed, so the pigs settle from there.
    pig_landed_ms: u64,
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
    /// Filtered gravity-up direction in die coordinates (milli-g); `None`
    /// until the first IMU sample.
    up: Option<[f32; 3]>,
    orientation: TextOrientation,
    gravity: orientation::Gravity,
    touch_since: Option<u64>,
    /// The die moved while a finger was on it: a grab, not a tap.
    touch_disturbed: bool,
    /// How long the die had been resting when the touch began.
    touch_rested_ms: u64,
    /// Since when the die has been resting, if it is.
    still_since: Option<u64>,
    /// The last tap was short, on a resting die, and the die stayed put.
    tap_deliberate: bool,
    /// A bank that just locked in, and when.
    locked: Option<(pigs::Locked, u64)>,
    /// The face a touch started on (the lowest one, if several).
    touch_face: Face,
    menu_hold_fired: bool,
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
            addr_of_mut!((*p).hw).write(hw);
            addr_of_mut!((*p).sm).write(StateMachine::new());
            addr_of_mut!((*p).motion).write(MotionDetector::new());
            addr_of_mut!((*p).settings).write(settings);
            addr_of_mut!((*p).potato).write(Potato::new());
            addr_of_mut!((*p).sessions).write(Sessions::default());
            addr_of_mut!((*p).pig_clock).write(0.0);
            addr_of_mut!((*p).pig_land).write(0.0);
            addr_of_mut!((*p).pig_landed_ms).write(0);
            addr_of_mut!((*p).roller).write(roller);
            addr_of_mut!((*p).pending_special).write(None);
            addr_of_mut!((*p).last_tick_ms).write(None);
            addr_of_mut!((*p).ui).write(Ui::new());
            addr_of_mut!((*p).up).write(None);
            addr_of_mut!((*p).orientation).write(TextOrientation::new());
            addr_of_mut!((*p).gravity).write(orientation::Gravity::new());
            addr_of_mut!((*p).touch_since).write(None);
            addr_of_mut!((*p).touch_disturbed).write(false);
            addr_of_mut!((*p).touch_rested_ms).write(0);
            addr_of_mut!((*p).still_since).write(None);
            addr_of_mut!((*p).tap_deliberate).write(false);
            addr_of_mut!((*p).locked).write(None);
            addr_of_mut!((*p).touch_face).write(Face::PosZ);
            addr_of_mut!((*p).menu_hold_fired).write(false);
            addr_of_mut!((*p).menu).write(None);
            addr_of_mut!((*p).up_face).write(Face::PosY);
            addr_of_mut!((*p).frozen).write(None);
            addr_of_mut!((*p).last_roll).write(None);
            addr_of_mut!((*p).docked).write(false);
            addr_of_mut!((*p).nest).write(Nest::new());
            addr_of_mut!((*p).reduced_motion).write(false);
            addr_of_mut!((*p).last_activity).write(0);
            addr_of_mut!((*p).asleep).write(false);
        }
        #[allow(unused_variables)]
        fn _fields<P: Platform>(f: Firmware<P>) {
            let Firmware {
                hw,
                sm,
                motion,
                settings,
                potato,
                sessions,
                pig_clock,
                pig_land,
                pig_landed_ms,
                roller,
                fx,
                pending_special,
                last_tick_ms,
                fonts,
                ui,
                frames,
                layer,
                panel,
                up,
                orientation,
                gravity,
                touch_since,
                touch_disturbed,
                touch_rested_ms,
                still_since,
                tap_deliberate,
                locked,
                touch_face,
                menu_hold_fired,
                menu,
                up_face,
                frozen,
                last_roll,
                docked,
                nest,
                reduced_motion,
                last_activity,
                asleep,
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

    /// The phone picks which licensed modes the Mode page offers. If the
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
        self.sm.set_rolls(self.settings.play().rolls());
        self.potato = Potato::new();
        // A game in play carries on through the menu and other modes; it
        // ends only from its End game page. Pig Toss needs one to play, so
        // arriving without one (a fresh die, settings from the phone) starts
        // one for the table as set.
        self.locked = None;
        if self.settings.play() == PlayMode::PigToss && !self.sessions.live(ModeId::PigToss) {
            self.sessions.start_pigs(self.settings.players);
        }
    }

    /// The Pig Toss game: scores, whose turn and the last throw.
    pub fn pigs(&self) -> &Pigs {
        &self.sessions.pigs
    }

    /// The Hot Potato round in play, if any.
    pub fn potato(&self) -> &Potato {
        &self.potato
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
        let game = !self.settings.play().rolls();
        for event in events {
            if game && !matches!(self.sm.mode(), Mode::Menu | Mode::Nest | Mode::Off) {
                match event {
                    Event::Motion(Motion::Shaking) if self.potato.is_idle() => self.light_potato(now)?,
                    Event::Tap => {
                        for cmd in self.potato.tap(now) {
                            self.run_potato(cmd)?;
                        }
                    }
                    // No opening the menu mid-round.
                    Event::LongPress if self.potato.is_lit() => continue,
                    _ => {}
                }
            }
            // Only a deliberate tap on the top screen banks: short, on a die
            // that was resting and stays put. Picking the die up to throw
            // it puts a finger on that screen too, and mustn't pass the turn.
            if event == Event::Tap
                && self.settings.play() == PlayMode::PigToss
                && self.touch_face == self.up_face
                && self.tap_deliberate
                && matches!(self.sm.mode(), Mode::Idle | Mode::Reveal { .. })
            {
                self.bank_pigs(now)?;
            }
            if event == Event::Docked(true) {
                self.locked = None;
                for cmd in self.potato.reset() {
                    self.run_potato(cmd)?;
                }
            }
            if event == Event::Tap
                && self.settings.play() == PlayMode::PassThePot
                && self.tap_deliberate
                && matches!(self.sm.mode(), Mode::Idle | Mode::Reveal { .. })
            {
                self.tap_bills(now)?;
            }
            if event == Event::Tap {
                self.ui.tap(now, self.sm.mode());
            }
            let commands = self.sm.handle(event, now);
            for cmd in commands {
                self.execute(cmd, now)?;
            }
        }
        for cmd in self.potato.tick(now) {
            self.run_potato(cmd)?;
        }
        if self.potato.is_lit() {
            let heat = self.potato.heat(now);
            if let Some(smoke) = self.fx.smoke() {
                smoke.smolder(SMOLDER_MIN + (SMOLDER_MAX - SMOLDER_MIN) * heat);
            }
        }
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

    /// A shake lit Hot Potato: pick the fuse at random within the setting.
    fn light_potato(&mut self, now: u64) -> HalResult<()> {
        let range = self.settings.fuse.range_ms();
        let fuse = potato::fuse_from(self.hw.rng.next_u32()?, range);
        self.ui.game_started();
        for cmd in self.potato.light(now, fuse, range.1) {
            self.run_potato(cmd)?;
        }
        Ok(())
    }

    /// A throw landed in Pig Toss: draw both pigs' poses. It is a game,
    /// not a signed roll, so the roll chain stays as it was.
    fn throw_pigs(&mut self) -> HalResult<()> {
        let poses = [
            pigs::Pose::from_random(self.hw.rng.next_u32()?),
            pigs::Pose::from_random(self.hw.rng.next_u32()?),
        ];
        // A third word decides whether the pigs landed touching.
        let touching = pigs::smooch_from_random(self.hw.rng.next_u32()?);
        let throw = self.sessions.pigs.throw(poses, touching);
        self.locked = None;
        self.pending_special = None;
        if self.settings.haptics_on() {
            let effect = match throw.outcome {
                _ if throw.won => smokebomb_hal::HapticEffect::MaxCelebration,
                pigs::Outcome::Smooch => smokebomb_hal::HapticEffect::Dud,
                pigs::Outcome::Bust => smokebomb_hal::HapticEffect::Buzz,
                pigs::Outcome::Score(_) => smokebomb_hal::HapticEffect::Tick,
            };
            self.hw.haptics.play(effect)?;
        }
        Ok(())
    }

    /// A tap in Pass the Pot between rolls. The first puts the last result
    /// away and shows the bills screen; a tap while that's up changes how
    /// many bills the next roll uses, 3 → 2 → 1 → 3. Only deliberate taps
    /// count, so picking the die up doesn't change it.
    fn tap_bills(&mut self, now: u64) -> HalResult<()> {
        if self.ui.label_up(now) {
            let n = self.settings.pot_count;
            self.settings.pot_count = if n <= 1 { MAX_POT_DICE as u8 } else { n - 1 };
            if self.settings.haptics_on() {
                self.hw.haptics.play(smokebomb_hal::HapticEffect::Tick)?;
            }
        } else {
            self.ui.dismiss_result();
        }
        Ok(())
    }

    /// A tap in Pig Toss: bank the turn and pass the die, or start a new
    /// game once the win screen is up.
    fn bank_pigs(&mut self, now: u64) -> HalResult<()> {
        // A winning throw is still counting up: don't wipe it before the
        // win screen has had its moment.
        if self.sessions.pigs.last().is_some_and(|t| t.won) && self.pig_win_t(now).is_none() {
            return Ok(());
        }
        let players = self.sessions.pigs.players();
        let locked = match self.sessions.pigs.bank() {
            pigs::Banked::Passed {
                player,
                points,
                total,
            } => Some(pigs::Locked {
                player,
                points,
                before: total - points,
                after: total,
                next: (player + 1) % players,
            }),
            pigs::Banked::Nothing | pigs::Banked::NewGame => None,
        };
        self.locked = locked.map(|l| (l, now));
        if locked.is_some() && self.settings.haptics_on() {
            // A thunk as it locks.
            self.hw.haptics.play(smokebomb_hal::HapticEffect::LandingThud)?;
        }
        Ok(())
    }

    /// Seconds since the winning throw's score gave way to the win screen,
    /// once it has.
    fn pig_win_t(&self, now: u64) -> Option<f32> {
        let since_landing = now.saturating_sub(self.pig_landed_ms) as f32 / 1000.0;
        (self.settings.play() == PlayMode::PigToss
            && self.sessions.pigs.last().is_some_and(|t| t.won)
            && matches!(self.sm.mode(), Mode::Idle | Mode::Reveal { .. })
            && since_landing >= screens::pig_win::AT)
            .then_some(since_landing - screens::pig_win::AT)
    }

    /// Do what the game asked for: haptics and smoke.
    fn run_potato(&mut self, cmd: PotatoCommand) -> HalResult<()> {
        match cmd {
            PotatoCommand::Ignite => {
                self.hw.haptics.play(smokebomb_hal::HapticEffect::Tick)?;
                if let Some(smoke) = self.fx.smoke() {
                    smoke.smolder(SMOLDER_MIN);
                }
            }
            PotatoCommand::Tick => self.hw.haptics.play(smokebomb_hal::HapticEffect::Tick)?,
            PotatoCommand::Boom => {
                self.hw.haptics.play(smokebomb_hal::HapticEffect::Buzz)?;
                // A full cloud that drains over the faces, with embers.
                if let Some(smoke) = self.fx.smoke() {
                    smoke.throw();
                    smoke.land();
                }
            }
            PotatoCommand::Clear => {
                if let Some(smoke) = self.fx.smoke() {
                    smoke.clear();
                }
            }
        }
        Ok(())
    }

    /// The Sleep after setting: the screens go dark once the die has sat
    /// idle that long, and light again (with the setup label, no boot) when
    /// it is touched, picked up or thrown.
    fn update_sleep(&mut self, now: u64, input: bool, mode: &Mode) -> HalResult<()> {
        let busy = input
            || self.touch_since.is_some()
            || !matches!(mode, Mode::Idle)
            || !self.potato.is_idle()
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
        self.step_pigs(dt, before, after, now);
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

    /// Run the pigs' tumble while the die is shaken or thrown, and note when
    /// it lands so they can settle from there.
    fn step_pigs(&mut self, dt: f32, before: &Mode, after: &Mode, now: u64) {
        let flying = |m: &Mode| matches!(m, Mode::Shaking | Mode::Airborne | Mode::Settling);
        if flying(after) {
            if !flying(before) {
                self.pig_clock = 0.0;
            }
            // Rattling in the hand is quicker than the tumble.
            self.pig_clock += dt * if matches!(after, Mode::Shaking) { 1.7 } else { 1.0 };
        }
        if matches!(after, Mode::Reveal { .. }) && !matches!(before, Mode::Reveal { .. }) {
            self.pig_land = self.pig_clock;
            self.pig_landed_ms = now;
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
        // Grip rejection: touches while the die is moving are ignored. In
        // the menu the die is in the hand anyway; only a tip in progress
        // blocks a hold.
        let usable = mask != 0
            && match &self.menu {
                Some(m) => m.turning.is_none(),
                None => self.motion.is_still(),
            };
        // A finger on a die that starts to move is a grab.
        if mask != 0 && !self.motion.is_still() {
            self.touch_disturbed = true;
        }
        let mut started = false;
        match (usable, self.touch_since) {
            (true, None) => {
                self.touch_since = Some(now);
                self.touch_disturbed = false;
                self.touch_rested_ms = self.still_since.map_or(0, |s| now.saturating_sub(s));
                self.nest.touch(now);
                self.touch_face = Face::ALL[mask.trailing_zeros() as usize % Face::ALL.len()];
                self.menu_hold_fired = false;
                started = true;
            }
            // Held for more than 0.8 s: at 60 Hz that's 49 frames, as in the
            // mockup, whose float clock never quite reaches 0.8 after 48.
            (true, Some(since)) if !self.menu_hold_fired && now.saturating_sub(since) > MENU_HOLD_MS => {
                self.menu_hold_fired = true;
                // In the menu a hold saves, unless the draft has another
                // step first.
                let next = self
                    .menu
                    .as_ref()
                    .is_some_and(|m| matches!(m.draft.held(), Held::Next(_)));
                let _ = events.push(if next { Event::MenuNext } else { Event::LongPress });
            }
            (false, Some(since)) => {
                self.tap_deliberate = !self.touch_disturbed
                    && now.saturating_sub(since) <= TAP_MAX_MS
                    && self.touch_rested_ms >= TAP_REST_MS;
                // Letting go after a hold that saved the menu shows the
                // setup like a tap does (the mockup's pointer-up); letting go
                // after the hold that opened it doesn't. A tap inside the
                // menu changes the selected setting, or powers the die off.
                if !self.menu_hold_fired || self.menu.is_none() {
                    let off = self.menu.as_ref().is_some_and(|m| m.draft.power_off_selected());
                    let _ = events.push(if off { Event::PowerOff } else { Event::Tap });
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
        let held = now.saturating_sub(since);
        let ring = !self.menu_hold_fired
            && !self.potato.is_lit()
            && held > HOLD_RING_AFTER_MS
            && matches!(
                self.sm.mode(),
                Mode::Idle | Mode::Reveal { .. } | Mode::Menu | Mode::Nest
            );
        ring.then(|| {
            ((held - HOLD_RING_AFTER_MS) as f32 / (MENU_HOLD_MS - HOLD_RING_AFTER_MS) as f32).min(1.0)
        })
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
            Command::Roll if self.settings.play() == PlayMode::PigToss => {
                self.throw_pigs()?;
            }
            Command::Roll => {
                let (die, count) = self.settings.active();
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
                self.potato = Potato::new();
                self.locked = None;
                let up = self.up.unwrap_or([0.0, 1.0, 0.0]);
                let live = self.sessions.live(ModeId::PigToss);
                self.menu = Some(MenuSession {
                    draft: Draft::new(&self.settings).with_session(live),
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
            Command::MenuTap => {
                if let Some(m) = &mut self.menu {
                    let next = m.draft.tapped();
                    if next != m.draft && self.settings.haptics_on() {
                        self.hw.haptics.play(smokebomb_hal::HapticEffect::Tick)?;
                    }
                    m.draft = next;
                    m.last_input = now;
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
                            self.sessions.end(ModeId::PigToss);
                        }
                        if m.draft.set_up() {
                            self.sessions.start_pigs(m.draft.players);
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
            .sessions
            .pigs
            .winner()
            .unwrap_or(self.sessions.pigs.current());
        let pigs_won = self.sessions.pigs.winner().is_some();
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
        // Pig Toss: the pigs tumble while the die is in the air, and settle
        // into their poses once it lands, for as long as the result is up.
        let since_landing = now.saturating_sub(self.pig_landed_ms) as f32 / 1000.0;
        let players = self.sessions.pigs.players();
        // A winning throw's score gives way to the win screen.
        let pig_win = self.pig_win_t(now).and_then(|t| {
            let fade = (LOCKED_MS as f32 / 1000.0 - t).clamp(0.0, 1.5) / 1.5;
            let winner = self.sessions.pigs.winner()?;
            (fade > 0.0).then(|| {
                (
                    tokens[winner as usize],
                    self.sessions.pigs.scores()[winner as usize],
                    t,
                    fade,
                )
            })
        });
        let win_up = pig_win.is_some();
        let pig_scene = if !pigs_on || win_up || matches!(mode, Mode::Menu | Mode::Nest | Mode::Off) {
            None
        } else if matches!(mode, Mode::Shaking | Mode::Airborne | Mode::Settling) {
            Some(pigfx::Scene::Tumbling(self.pig_clock))
        } else {
            self.sessions
                .pigs
                .last()
                .filter(|_| self.ui.showing_result(now))
                .map(|t| pigfx::Scene::Settling {
                    land: self.pig_land,
                    u: since_landing / pigfx::SETTLE_S,
                    shrink: ease_inout(
                        (since_landing - screens::pig_score::SHRINK_AT) / screens::pig_score::SHRINK_S,
                    ),
                    poses: t.poses,
                    touching: t.touching,
                })
        };
        let pig_alpha = if matches!(mode, Mode::Shaking | Mode::Airborne | Mode::Settling) {
            1.0
        } else {
            self.ui.result_dim(now) * screens::pig_win::score_fade(self.sessions.pigs.last(), since_landing)
        };
        // A bank locking in: a padlock snaps shut and the score counts up.
        let locked = self
            .locked
            .filter(|_| pigs_on && pig_scene.is_none() && matches!(mode, Mode::Idle | Mode::Reveal { .. }))
            .filter(|(_, at)| now.saturating_sub(*at) < LOCKED_MS)
            .map(|(l, at)| (l, now.saturating_sub(at) as f32 / 1000.0));
        let hold = self.hold_progress(now).map(|p| (self.touch_face, p));
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
            sessions,
            fx,
            ..
        } = self;
        let record = last_roll.as_ref().map(|r| &r.record);
        let pigs_throw = if pigs_on {
            sessions.pigs.last().copied()
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
                pigs.draw(scene);
            }
        }

        for face in Face::ALL {
            let fb = &mut frames[face.index()];
            fb.clear();
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
            let content = if potato_face.is_some() || locked.is_some() || win_up {
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
                FaceContent::Boot { t, top } => TargetOf::<P>::draw_boot(&mut c, face.index(), top, t),
                FaceContent::Wake { alpha } if pigs_on => {
                    screens::draw_pigs_label(&mut c, setup, tokens[pigs_up as usize], pigs_won, alpha)
                }
                FaceContent::Wake { alpha } => {
                    TargetOf::<P>::draw_wake_label(&mut c, setup, &label, alpha, face.index())
                }
                FaceContent::Result { alpha } => {
                    if let Some(t) = pigs_throw {
                        let next = (t.player + 1) % players;
                        let alpha = alpha * screens::pig_win::score_fade(Some(&t), since_landing);
                        screens::draw_pig_score(&mut c, &t, since_landing, next, &tokens, alpha);
                    } else if let Some(r) = record {
                        TargetOf::<P>::draw_result(&mut c, r, ui.special(), alpha);
                    }
                }
                FaceContent::Success { t, setup } => {
                    TargetOf::<P>::draw_success(&mut c, &setup.label(), setup.nudge(), t);
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
                    screens::draw_locked(
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
                    screens::draw_pig_win(&mut c, player, total, t, fade);
                }
            }
            if let Some(scene) = pig_scene {
                // In the air every face shows the pigs: which one faces down
                // changes all the time, and would blink on and off. Landed,
                // the face-down screen stays dark (H2).
                let in_air = matches!(scene, pigfx::Scene::Tumbling(_));
                if in_air || face != up.opposite() {
                    if let Some(pigs) = fx.pigs() {
                        pigs.lay_onto(fb_of(c.painter), rot, pig_alpha);
                    }
                }
            }
            match potato_face {
                Some(PotatoView::Fuse(heat, pulse)) => screens::draw_fuse(&mut c, heat, pulse),
                Some(PotatoView::Boom(t)) => screens::draw_boom(&mut c, t),
                None => {}
            }
        }

        // Smoke over everything, wrapping round the edges (SIM_SPEC D1).
        if let Some(smoke) = self.fx.smoke_ref() {
            smoke.draw(&mut self.frames);
        }
        for face in Face::ALL {
            TargetOf::<P>::pack(&self.frames[face.index()], &mut self.panel);
            self.hw.display.write_frame(face, &self.panel)?;
        }
        self.hw.display.flush()
    }
}

/// How much of a full cloud a lit fuse's smoke may reach, at the start and
/// at full heat: enough to build, little enough to read "PASS IT" through.
const SMOLDER_MIN: f32 = 0.08;
const SMOLDER_MAX: f32 = 0.3;

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
        screens::draw_hold_ring(c, 1.0, intro.ring_alpha, intro.ring_grow);
        T::draw_menu(c, &m.draft, battery, 0.0, 0.0, intro.alpha, intro.scale);
        if let Some(p) = hold {
            screens::draw_hold_ring(c, p, 1.0, 0.0);
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

/// 0 to 1 with a soft start and end; clamped outside.
fn ease_inout(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
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
