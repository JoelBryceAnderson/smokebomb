//! On-die setup menu (SIM_SPEC C3).
//!
//! Hold any screen to open the menu there. Tipping the die moves through it:
//! left and right turn the page, up and down change the value. Taps do
//! nothing here (brief 3, part 2). Everything happens on a [`Draft`]; a hold
//! saves it and returns to the roll, and a throw, docking or 25 s without
//! input leaves the setup as it was.
//!
//! The menu is the current app's pages with **Apps** one tip right of them
//! (docs/APP_FRAMEWORK.md). Apps lists the games and Settings. Settings is an
//! app of its own that doesn't take over the die: a hold on it goes into its
//! pages (Battery, then one page per setting, Legal and Power), and saving
//! there goes back to the game in use. A hold on Power powers the die off.
//! Every value has a short name that fits one line on the 64×64 panel.
//!
//! Pig Toss asks a little more of a hold ([`Draft::held`]). A game in play
//! is a session ([`crate::session`]) that the menu and other modes leave
//! alone, so while one is live Pig Toss has an End game page where Players
//! would be, and a hold there ends it. Without a game, choosing Pig Toss or
//! holding on Players walks through setting one up: how many players, then
//! each player's initial or symbol, one hold per step. Nothing ends or
//! starts until the menu saves.
//!
//! A game can also open the menu on one page alone ([`Draft::alone`]) to
//! adjust one thing (brief 3, 2.2.4): Pass the Pot's Bills in hand, or after
//! a Pig Toss win, Next game (a rematch, or setting up Players again).

use core::fmt::Write as _;

use heapless::String;
use smokebomb_shared::types::{MAX_DICE, MAX_POT_DICE};
use smokebomb_shared::{DieKind, ModeId, ModeSet};

use crate::pigs::{Token, DEFAULT_TOKENS, MAX_PLAYERS};
use crate::smoke::Amount;
use crate::tips::TipDir;

/// One setting: a page of its own in the Settings app, whose value tips up
/// and down through `options`.
pub struct Item {
    pub name: &'static str,
    /// Short names, each a line on the smallest panel.
    pub options: &'static [&'static str],
    /// Which option a fresh die starts on.
    pub default: u8,
}

impl Item {
    const fn choice(name: &'static str, options: &'static [&'static str], default: u8) -> Self {
        Self {
            name,
            options,
            default,
        }
    }
}

/// The settings. Owner and About are the phone's (the mockup also had
/// Large text, Night mode and Verified rolls: SIM_SPEC H13).
pub const SETTINGS: [Item; 5] = [
    Item::choice("Brightness", &["30%", "50%", "70%", "100%"], 2),
    Item::choice("Haptics", &["Off", "On"], 1),
    Item::choice("Sugar", &["Off", "Lite", "Full"], 2),
    Item::choice("Sleep", &["30s", "1m", "2m", "5m", "Off"], 2),
    Item::choice("Bluetooth", &["Off", "On"], 1),
];

const BRIGHTNESS: usize = 0;
const HAPTICS: usize = 1;
const SMOKE: usize = 2;
const SLEEP: usize = 3;
const BLUETOOTH: usize = 4;

/// The Legal page's entries, a label over its value, tipped through.
/// Electronic labelling (FCC 47 CFR 2.935, ISED RSS-Gen) needs them on the
/// die's own screen. The numbers are placeholders until it is certified.
pub const LEGAL: [(&str, &str); 4] = [
    ("FCC ID", "TBD"),
    ("IC", "TBD"),
    ("Model", "SC-1"),
    ("Marks", "CE"),
];

/// The chosen option of every setting.
pub type Choices = [u8; SETTINGS.len()];

fn default_choices() -> Choices {
    let mut c = [0; SETTINGS.len()];
    for (c, item) in c.iter_mut().zip(&SETTINGS) {
        *c = item.default;
    }
    c
}

/// What the die is being used for. Dice keeps the count and die pages;
/// each game brings its own options page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    Dice,
    PassThePot,
    HotPotato,
    /// Two pigs to throw for points; the die keeps score for the table.
    PigToss,
}

impl PlayMode {
    /// Order on the Apps page.
    pub const ALL: [PlayMode; 4] = [
        PlayMode::Dice,
        PlayMode::PassThePot,
        PlayMode::HotPotato,
        PlayMode::PigToss,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            PlayMode::Dice => "Dice",
            PlayMode::PassThePot => "Pass the Pot",
            PlayMode::HotPotato => "Hot Potato",
            PlayMode::PigToss => "Pig Toss",
        }
    }

    /// The name on the Apps page: one short line.
    pub const fn short_name(self) -> &'static str {
        match self {
            PlayMode::Dice => "Dice",
            PlayMode::PassThePot => "Pot",
            PlayMode::HotPotato => "Potato",
            PlayMode::PigToss => "Pigs",
        }
    }

    /// The mode's id on the wire and in the store.
    pub const fn id(self) -> ModeId {
        match self {
            PlayMode::Dice => ModeId::Dice,
            PlayMode::PassThePot => ModeId::PassThePot,
            PlayMode::HotPotato => ModeId::HotPotato,
            PlayMode::PigToss => ModeId::PigToss,
        }
    }

    pub const fn from_id(id: ModeId) -> Self {
        match id {
            ModeId::Dice => PlayMode::Dice,
            ModeId::PassThePot => PlayMode::PassThePot,
            ModeId::HotPotato => PlayMode::HotPotato,
            ModeId::PigToss => PlayMode::PigToss,
        }
    }

    /// The menu's pages in this mode: Apps, then the mode's own. Tipping
    /// left goes to the next one, so Apps is one tip right of the first.
    pub const fn ring(self) -> &'static [Page] {
        match self {
            PlayMode::Dice => &[Page::Apps, Page::Count, Page::Die],
            PlayMode::PassThePot => &[Page::Apps, Page::Pot],
            PlayMode::HotPotato => &[Page::Apps, Page::Fuse],
            PlayMode::PigToss => &[Page::Apps, Page::Players],
        }
    }

    /// The page the menu opens on: the mode's first page after Apps.
    pub const fn home(self) -> Page {
        self.ring()[1]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// The games and Settings.
    Apps,
    Count,
    Die,
    /// Pass the Pot's option: how many bills you hold, which is how many
    /// dice you roll (up to three).
    Pot,
    /// Hot Potato's option: how long the fuse may run.
    Fuse,
    /// Pig Toss' option: how many people are playing.
    Players,
    /// Pig Toss: player `n`'s (0-based) initial or symbol.
    Token(u8),
    /// Pig Toss while a game is in play: hold to end it. Players takes its
    /// place once it's ended.
    EndGame,
    /// Pig Toss once someone has won: a rematch for the same players, or
    /// back to setting up Players. Opened alone by a hold on the win screen.
    Next,
    /// Settings: the battery's charge, to read.
    Battery,
    /// Settings: one of [`SETTINGS`].
    Setting(u8),
    /// Settings: the approval numbers ([`LEGAL`]), to read.
    Legal,
    /// Settings: hold to power off.
    Power,
}

/// The pages of Pig Toss' setup: how many players, then each one's token.
/// The ring is the first `1 + players` of them.
static NAMING: [Page; 1 + crate::pigs::MAX_PLAYERS as usize] = [
    Page::Players,
    Page::Token(0),
    Page::Token(1),
    Page::Token(2),
    Page::Token(3),
    Page::Token(4),
    Page::Token(5),
];

/// Pig Toss' pages while a game is in play: End game where Players was.
const PIGS_LIVE: &[Page] = &[Page::Apps, Page::EndGame];

/// The Apps page alone, while Settings is chosen on it.
const APPS_ALONE: &[Page] = &[Page::Apps];

/// The Settings app's pages.
const SETTINGS_RING: &[Page] = &[
    Page::Battery,
    Page::Setting(0),
    Page::Setting(1),
    Page::Setting(2),
    Page::Setting(3),
    Page::Setting(4),
    Page::Legal,
    Page::Power,
];

/// The pages a game opens alone, to adjust one thing (brief 3, 2.2.4).
const POT_ALONE: &[Page] = &[Page::Pot];
const NEXT_ALONE: &[Page] = &[Page::Next];

impl Page {
    /// The page's title: one short line.
    pub const fn title(self) -> &'static str {
        match self {
            Page::Apps => "Apps",
            Page::Count => "How many",
            Page::Die => "Which die",
            Page::Pot => "Bills",
            Page::Fuse => "Fuse",
            Page::Players => "Players",
            Page::Token(i) => match i {
                0 => "Player 1",
                1 => "Player 2",
                2 => "Player 3",
                3 => "Player 4",
                4 => "Player 5",
                _ => "Player 6",
            },
            Page::EndGame => "End game",
            Page::Next => "Next game",
            Page::Battery => "Battery",
            Page::Setting(i) => SETTINGS[i as usize % SETTINGS.len()].name,
            Page::Legal => "Legal",
            Page::Power => "Power",
        }
    }
}

/// How long Hot Potato's fuse can run. The die picks a random time inside the
/// range each round, so nobody can count it down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fuse {
    Short,
    Medium,
    Long,
}

impl Fuse {
    pub const ALL: [Fuse; 3] = [Fuse::Short, Fuse::Medium, Fuse::Long];

    pub const fn name(self) -> &'static str {
        match self {
            Fuse::Short => "Fast",
            Fuse::Medium => "Mid",
            Fuse::Long => "Slow",
        }
    }

    /// The shortest and longest fuse, in milliseconds.
    pub const fn range_ms(self) -> (u32, u32) {
        match self {
            Fuse::Short => (10_000, 20_000),
            Fuse::Medium => (20_000, 40_000),
            Fuse::Long => (40_000, 90_000),
        }
    }
}

/// What a saved setup is, for the labels the die shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    /// Dice to roll: `d20`, `3d6`, `Pass the Pot ×2`.
    Roll(DieKind, u8),
    HotPotato,
    /// Pig Toss for this many players.
    Pigs(u8),
}

impl Setup {
    /// The wake label and the success screen's line.
    pub fn label(self) -> String<24> {
        match self {
            Setup::Roll(die, count) => crate::screens::setup_label(die, count),
            Setup::HotPotato => {
                let mut s = String::new();
                let _ = s.push_str("Hot Potato");
                s
            }
            Setup::Pigs(_) => {
                let mut s = String::new();
                let _ = s.push_str("Pig Toss");
                s
            }
        }
    }

    /// The line under the label on the success screen (an H2 line: ten
    /// characters at most).
    pub const fn nudge(self) -> &'static str {
        match self {
            Setup::Roll(..) => "Ready",
            Setup::HotPotato => "Shake it",
            Setup::Pigs(_) => "Ready",
        }
    }

    /// The menu's status bar: `3d6`, `Pot ×2`, `Potato`.
    pub fn short_label(self) -> String<24> {
        match self {
            Setup::Roll(die, count) => short_label(die, count),
            Setup::HotPotato => {
                let mut s = String::new();
                let _ = s.push_str("Potato");
                s
            }
            Setup::Pigs(players) => {
                let mut s = String::new();
                let _ = write!(s, "Pigs ×{players}");
                s
            }
        }
    }
}

/// User settings. Persisted to internal flash on hardware (TODO).
///
/// The dice setup and each game's options are kept apart, so switching modes
/// and back leaves `3d6` as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Modes this die holds a license for. Licenses are the die's, not
    /// preferences: replacing the settings keeps them.
    pub licensed: ModeSet,
    /// Modes the owner has turned on from the phone. With only Dice among
    /// the licensed ones, Apps holds Dice and Settings.
    pub enabled: ModeSet,
    pub play: PlayMode,
    pub die: DieKind,
    pub count: u8,
    /// Pass the Pot: the bills in your hand, which is how many dice you
    /// roll. Everyone starts with three.
    pub pot_count: u8,
    pub fuse: Fuse,
    /// Pig Toss: how many people are playing.
    pub players: u8,
    /// Pig Toss: each player's initial or symbol.
    pub tokens: [Token; MAX_PLAYERS as usize],
    /// The chosen option of each [`SETTINGS`] item.
    pub choices: Choices,
    /// A short id made from the die's serial, which About shows. It is the
    /// die's identity, not a preference: the firmware fills it in from the
    /// secure element, and loading saved settings must not replace it.
    pub device_id: u16,
    /// Night hours, local: the Nest's screens are off from the first hour to
    /// the second (23 to 7 by default). The phone app sets them; the menu
    /// has no item for them.
    pub night_hours: (u8, u8),
}

/// A 16-bit id from a die's serial (FNV-1a folded), for About: every byte
/// counts, and the same serial always gives the same id.
pub fn short_id(serial: &[u8]) -> u16 {
    let mut h: u32 = 0x811c_9dc5;
    for &b in serial {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    ((h >> 16) ^ h) as u16
}

impl Default for Settings {
    fn default() -> Self {
        // The mockup's default setup: a single d20.
        Self {
            // Every mode so far ships free; store modes will start unlicensed.
            licensed: ModeSet::ALL,
            enabled: ModeSet::ALL,
            play: PlayMode::Dice,
            die: DieKind::D20,
            count: 1,
            pot_count: 3,
            fuse: Fuse::Medium,
            players: crate::pigs::MIN_PLAYERS,
            tokens: DEFAULT_TOKENS,
            choices: default_choices(),
            device_id: 0,
            night_hours: (crate::nest::NIGHT_START_H, crate::nest::NIGHT_END_H),
        }
    }
}

impl Settings {
    /// Screen brightness, percent.
    pub fn brightness_pct(&self) -> u8 {
        const PCT: [u8; 4] = [30, 50, 70, 100];
        PCT[self.choices[BRIGHTNESS] as usize % PCT.len()]
    }

    /// Haptics off: the die stays silent.
    pub fn haptics_on(&self) -> bool {
        self.choices[HAPTICS] != 0
    }

    pub fn bluetooth_on(&self) -> bool {
        self.choices[BLUETOOTH] != 0
    }

    /// How much smoke the die makes.
    pub fn smoke_amount(&self) -> Amount {
        [Amount::Off, Amount::Light, Amount::Full][self.choices[SMOKE] as usize % 3]
    }

    /// How long the die may sit untouched before its screens go dark, or
    /// `None` for never.
    pub fn sleep_after_ms(&self) -> Option<u64> {
        const MS: [Option<u64>; 5] = [Some(30_000), Some(60_000), Some(120_000), Some(300_000), None];
        MS[self.choices[SLEEP] as usize % MS.len()]
    }

    /// The games the Apps page offers: licensed and turned on. Dice is
    /// always among them.
    pub fn modes(&self) -> ModeSet {
        self.licensed.intersect(self.enabled).with(ModeId::Dice)
    }

    /// The mode in use: Dice when the saved one is no longer offered.
    pub fn play(&self) -> PlayMode {
        if self.modes().contains(self.play.id()) {
            self.play
        } else {
            PlayMode::Dice
        }
    }

    /// The die and count a throw rolls in the current mode. A game that
    /// doesn't roll leaves this at the dice setup.
    pub fn active(&self) -> (DieKind, u8) {
        match self.play() {
            PlayMode::Dice | PlayMode::HotPotato | PlayMode::PigToss => (self.die, self.count),
            PlayMode::PassThePot => (DieKind::PassThePot, self.pot_count),
        }
    }

    /// What the die tells the person it's set up for.
    pub fn setup(&self) -> Setup {
        match self.play() {
            PlayMode::HotPotato => Setup::HotPotato,
            PlayMode::PigToss => Setup::Pigs(self.players),
            _ => {
                let (die, count) = self.active();
                Setup::Roll(die, count)
            }
        }
    }
}

/// The menu's working copy of the setup, plus where the menu is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draft {
    pub page: Page,
    /// The games the Apps page offers ([`Settings::modes`]).
    pub modes: ModeSet,
    pub play: PlayMode,
    /// The mode in use when the menu opened: Settings goes back to it.
    pub home: PlayMode,
    /// On the Apps page, Settings is chosen rather than a game.
    pub on_settings: bool,
    /// In the Settings app: the ring is its pages.
    pub in_settings: bool,
    pub die: DieKind,
    pub count: u8,
    pub pot_count: u8,
    pub fuse: Fuse,
    pub players: u8,
    pub tokens: [Token; MAX_PLAYERS as usize],
    pub choices: Choices,
    /// The [`LEGAL`] entry shown.
    pub legal: u8,
    pub device_id: u16,
    /// In Pig Toss' setup: the ring is its pages ([`NAMING`]).
    pub naming: bool,
    /// A Pig Toss game is in play (and hasn't been ended in this draft).
    pub live: bool,
    /// The game in play was ended: saving ends it.
    pub ended: bool,
    /// Opened by a game on one page, to adjust one thing: the ring is that
    /// page alone.
    pub alone: bool,
    /// On [`Page::Next`]: a rematch, rather than setting up Players again.
    pub rematch: bool,
}

/// What a hold in the menu does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// Go on to the next step, still in the menu.
    Next(Draft),
    /// Save and close.
    Save,
    /// Power the die off, saving nothing.
    PowerOff,
}

impl Draft {
    /// The menu opens on the current mode's first page.
    pub fn new(s: &Settings) -> Self {
        Self {
            page: s.play().home(),
            modes: s.modes(),
            play: s.play(),
            home: s.play(),
            on_settings: false,
            in_settings: false,
            die: s.die,
            count: s.count,
            pot_count: s.pot_count,
            fuse: s.fuse,
            players: s.players,
            tokens: s.tokens,
            choices: s.choices,
            legal: 0,
            device_id: s.device_id,
            naming: false,
            live: false,
            ended: false,
            alone: false,
            rematch: true,
        }
    }

    /// A draft opened by a game on `page` alone (Pass the Pot's bills, or
    /// what to play after a win).
    pub fn alone(s: &Settings, page: Page) -> Self {
        Self {
            page,
            alone: true,
            ..Self::new(s)
        }
    }

    /// Saving this draft starts the same Pig Toss game again from 0.
    pub fn restart(&self) -> bool {
        self.page == Page::Next && self.rematch && !self.naming
    }

    /// The same draft, knowing whether a Pig Toss game is in play. With
    /// one, Pig Toss opens on Apps, a tip away from End game, so a stray
    /// hold doesn't land on ending it.
    pub fn with_session(self, live: bool) -> Self {
        let mut d = Self { live, ..self };
        if live && d.play == PlayMode::PigToss && d.page == Page::Players {
            d.page = Page::Apps;
        }
        d
    }

    /// A hold here powers the die off.
    pub fn powers_off(&self) -> bool {
        self.in_settings && self.page == Page::Power
    }

    /// Saving this draft sets up a new Pig Toss game: it went through the
    /// setup to the last player.
    pub fn set_up(&self) -> bool {
        self.play == PlayMode::PigToss && self.naming
    }

    /// The pages the draft can tip through.
    pub fn ring(&self) -> &'static [Page] {
        if self.naming {
            &NAMING[..1 + self.players as usize]
        } else if self.alone {
            match self.page {
                Page::Pot => POT_ALONE,
                _ => NEXT_ALONE,
            }
        } else if self.in_settings {
            SETTINGS_RING
        } else if self.on_settings {
            APPS_ALONE
        } else if self.play == PlayMode::PigToss && self.live {
            PIGS_LIVE
        } else {
            self.play.ring()
        }
    }

    /// A hold: save, or in Pig Toss go on to the next step first. On End
    /// game the game ends and Players comes back. Without a game, Pig Toss
    /// can't be saved until one is set up: a hold goes to Players, a hold
    /// on Players to the first player's token, each token to the next, and
    /// the last saves.
    pub fn held(self) -> Held {
        use crate::menu::Page::*;
        if self.in_settings {
            return if self.page == Power {
                Held::PowerOff
            } else {
                Held::Save
            };
        }
        if self.on_settings {
            // Into the Settings app, with the game in use as it was.
            return Held::Next(Self {
                page: Battery,
                play: self.home,
                on_settings: false,
                in_settings: true,
                ..self
            });
        }
        if self.page == Next && self.rematch {
            return Held::Save;
        }
        if self.play != PlayMode::PigToss {
            return Held::Save;
        }
        let mut next = self;
        next.page = match self.page {
            Next => Players,
            EndGame => {
                next.live = false;
                next.ended = true;
                return Held::Next(Self {
                    page: Players,
                    ..next
                });
            }
            Token(i) if i + 1 < self.players => Token(i + 1),
            Token(_) => return Held::Save,
            Players => Token(0),
            _ if self.live || self.naming => return Held::Save,
            _ => Players,
        };
        next.naming = true;
        Held::Next(next)
    }

    /// Where the page sits in this mode's ring, for the page dots.
    pub fn page_index(&self) -> usize {
        self.ring().iter().position(|p| *p == self.page).unwrap_or(0)
    }

    /// The draft after a tip: left is the next page, right the previous; up
    /// is the next value, down the previous. Values wrap around.
    pub fn tipped(self, dir: TipDir) -> Self {
        let mut next = self;
        let ring = self.ring();
        match dir {
            TipDir::Left => next.page = ring[step(self.page_index(), 1, ring.len())],
            TipDir::Right => next.page = ring[step(self.page_index(), -1, ring.len())],
            TipDir::Up | TipDir::Down => {
                let by = if dir == TipDir::Up { 1 } else { -1 };
                match self.page {
                    Page::Apps => {
                        // The games offered, then Settings.
                        let mut offered = heapless::Vec::<PlayMode, 8>::new();
                        for m in PlayMode::ALL.into_iter().filter(|m| self.modes.contains(m.id())) {
                            let _ = offered.push(m);
                        }
                        let n = offered.len() + 1;
                        let i = if self.on_settings {
                            offered.len()
                        } else {
                            offered.iter().position(|m| *m == self.play).unwrap_or(0)
                        };
                        let j = step(i, by, n);
                        next.on_settings = j == offered.len();
                        if let Some(&m) = offered.get(j) {
                            next.play = m;
                        }
                    }
                    Page::Count => {
                        next.count = step(self.count as usize - 1, by, MAX_DICE) as u8 + 1;
                    }
                    Page::Die => {
                        let i = DieKind::NUMERIC.iter().position(|d| *d == self.die).unwrap_or(0);
                        next.die = DieKind::NUMERIC[step(i, by, DieKind::NUMERIC.len())];
                    }
                    Page::Pot => {
                        next.pot_count = step(self.pot_count as usize - 1, by, MAX_POT_DICE) as u8 + 1;
                    }
                    Page::Fuse => {
                        let i = Fuse::ALL.iter().position(|f| *f == self.fuse).unwrap_or(0);
                        next.fuse = Fuse::ALL[step(i, by, Fuse::ALL.len())];
                    }
                    Page::Players => {
                        let n = (crate::pigs::MAX_PLAYERS - crate::pigs::MIN_PLAYERS + 1) as usize;
                        let i = (self.players - crate::pigs::MIN_PLAYERS) as usize;
                        next.players = step(i, by, n) as u8 + crate::pigs::MIN_PLAYERS;
                    }
                    Page::Token(i) => {
                        let t = &mut next.tokens[i as usize];
                        *t = t.stepped(by as i32);
                    }
                    Page::EndGame => {}
                    Page::Next => next.rematch = !self.rematch,
                    Page::Setting(i) => {
                        let (i, n) = (i as usize, SETTINGS[i as usize].options.len());
                        next.choices[i] = step(self.choices[i] as usize, by, n) as u8;
                    }
                    Page::Legal => next.legal = step(self.legal as usize, by, LEGAL.len()) as u8,
                    Page::Battery | Page::Power => {}
                }
            }
        }
        next
    }

    /// The draft after `steps` tips in `dir` (negative steps go the other
    /// way), as if tipped one face at a time.
    pub fn stepped(self, dir: TipDir, steps: i32) -> Self {
        let d = if steps < 0 { dir.opposite() } else { dir };
        (0..steps.unsigned_abs()).fold(self, |m, _| m.tipped(d))
    }

    /// A setting's current value, by its index in [`SETTINGS`].
    pub fn setting(&self, i: usize) -> &'static str {
        let item = &SETTINGS[i];
        item.options[(self.choices[i] as usize).min(item.options.len() - 1)]
    }

    pub fn commit(&self, s: &mut Settings) {
        s.play = self.play;
        s.die = self.die;
        s.count = self.count;
        s.pot_count = self.pot_count;
        s.fuse = self.fuse;
        s.players = self.players;
        s.tokens = self.tokens;
        s.choices = self.choices;
    }

    /// The setup the draft would save.
    pub fn setup(&self) -> Setup {
        let mut s = Settings {
            licensed: self.modes,
            enabled: self.modes,
            ..Settings::default()
        };
        self.commit(&mut s);
        s.setup()
    }

    /// The die and count the draft would roll.
    pub fn active(&self) -> (DieKind, u8) {
        let mut s = Settings {
            licensed: self.modes,
            enabled: self.modes,
            ..Settings::default()
        };
        self.commit(&mut s);
        s.active()
    }

    /// The page's main text: its value, or for the Apps page and the Legal
    /// page what's chosen.
    pub fn value(&self) -> String<16> {
        let view = self.view(0);
        let mut s = String::new();
        let _ = match view.value {
            Value::Text(t) => s.push_str(&t),
            Value::Token(t) => match (t.initial(), t.as_symbol()) {
                (Some(c), _) => s.push(c).map_err(|_| ()),
                (_, Some(sym)) => s.push_str(sym.name()),
                _ => Ok(()),
            },
            Value::App(_) => s.push_str(view.caption.unwrap_or("")),
            Value::Lines([a, _]) => s.push_str(a),
        };
        s
    }

    /// What the page shows, with the battery at `battery` percent (brief 3,
    /// 1.3: H2 lines of at most 10 characters, an H1 value that fits a
    /// line).
    pub fn view(&self, battery: u8) -> PageView {
        let text = |t: &str| {
            let mut s = String::new();
            let _ = s.push_str(t);
            Value::Text(s)
        };
        let number = |n: u32, suffix: &str| {
            let mut s = String::new();
            let _ = write!(s, "{n}{suffix}");
            Value::Text(s)
        };
        let mut caption = None;
        let mut arrows = true;
        let value = match self.page {
            Page::Apps => {
                let (icon, name) = if self.on_settings {
                    (AppIcon::Settings, "Settings")
                } else {
                    (AppIcon::Game(self.play), self.play.short_name())
                };
                caption = Some(name);
                Value::App(icon)
            }
            Page::Count => number(self.count as u32, ""),
            Page::Die => text(self.die.wire_name()),
            Page::Pot => number(self.pot_count as u32, ""),
            Page::Fuse => text(self.fuse.name()),
            Page::Players => number(self.players as u32, ""),
            Page::Token(i) => Value::Token(self.tokens[i as usize]),
            Page::EndGame => {
                arrows = false;
                caption = Some("Clears all");
                text("End")
            }
            Page::Next => {
                caption = Some(if self.rematch { "Rematch" } else { "Players" });
                text(if self.rematch { "Redo" } else { "New" })
            }
            Page::Battery => {
                arrows = false;
                number(battery.min(100) as u32, "%")
            }
            Page::Setting(i) => text(self.setting(i as usize)),
            Page::Legal => {
                let (label, number) = LEGAL[self.legal as usize % LEGAL.len()];
                Value::Lines([label, number])
            }
            Page::Power => {
                arrows = false;
                caption = Some("Hold: off");
                text("Off")
            }
        };
        let mut status = String::new();
        let _ = status.push_str(&self.setup().short_label());
        let mut title = String::new();
        let _ = match self.page {
            Page::Token(i) => write!(title, "Player {}", i + 1),
            p => title.push_str(p.title()).map_err(|_| core::fmt::Error),
        };
        PageView {
            status,
            title,
            value,
            caption,
            arrows,
            dots: self.ring().len(),
            dot: self.page_index(),
        }
    }
}

/// An app's picture on the Apps page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppIcon {
    Game(PlayMode),
    Settings,
}

/// A held page's value (H1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Text(String<8>),
    /// A player's initial or symbol.
    Token(Token),
    /// An app's picture, named by the caption.
    App(AppIcon),
    /// Two H2 lines instead (the Legal page: a label over its number).
    Lines([&'static str; 2]),
}

/// What a held menu page shows: everything a panel needs to draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageView {
    /// The setup in use: `3d6`, `Pigs ×2` (H2).
    pub status: String<16>,
    /// `How many` (H2).
    pub title: String<16>,
    pub value: Value,
    /// One line under the value (H2).
    pub caption: Option<&'static str>,
    /// The value tips up and down: show ▲ ▼.
    pub arrows: bool,
    /// Page dots: how many, and which is lit.
    pub dots: usize,
    pub dot: usize,
}

fn step(i: usize, by: isize, n: usize) -> usize {
    (i as isize + by).rem_euclid(n as isize) as usize
}

/// The setup as the menu's status bar shows it: `3d6`, `Pot ×2`.
pub fn short_label(die: DieKind, count: u8) -> String<24> {
    match die {
        DieKind::PassThePot => {
            let mut s = String::new();
            let _ = write!(s, "Pot ×{count}");
            s
        }
        d => crate::screens::setup_label(d, count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        Draft::new(&Settings::default())
    }

    fn pot() -> Draft {
        Draft::new(&Settings {
            play: PlayMode::PassThePot,
            ..Settings::default()
        })
    }

    fn pigs(players: u8) -> Settings {
        Settings {
            play: PlayMode::PigToss,
            players,
            ..Settings::default()
        }
    }

    /// Hold on `d`, expecting to go on to another page.
    fn hold(d: Draft) -> Draft {
        match d.held() {
            Held::Next(next) => next,
            other => panic!("{other:?} on {:?}", d.page),
        }
    }

    /// The Apps page with Settings chosen, from the default menu.
    fn apps_on_settings() -> Draft {
        let apps = draft().tipped(TipDir::Right);
        apps.tipped(TipDir::Down)
    }

    /// The Settings app's first page.
    fn settings_app() -> Draft {
        hold(apps_on_settings())
    }

    #[test]
    fn dice_mode_opens_on_the_count_as_before() {
        let d = draft();
        assert_eq!(d.page, Page::Count);
        assert_eq!(d.tipped(TipDir::Up).count, 2);
        assert_eq!(d.tipped(TipDir::Left).page, Page::Die);
    }

    #[test]
    fn apps_is_one_tip_right_of_the_count() {
        let d = draft();
        assert_eq!(d.ring(), &[Page::Apps, Page::Count, Page::Die]);
        assert_eq!(d.tipped(TipDir::Right).page, Page::Apps);
        assert_eq!(d.stepped(TipDir::Left, 2).page, Page::Apps);
        assert_eq!(d.stepped(TipDir::Left, 3), d, "full circle");
    }

    #[test]
    fn up_and_down_change_the_value_and_wrap() {
        let d = draft();
        assert_eq!(d.tipped(TipDir::Down).count, 10);
        let die = d.tipped(TipDir::Left);
        assert_eq!(die.tipped(TipDir::Up).die, DieKind::D100);
        assert_eq!(die.tipped(TipDir::Down).die, DieKind::D12);
        let d100 = Draft {
            die: DieKind::D100,
            ..die
        };
        assert_eq!(
            d100.tipped(TipDir::Up).die,
            DieKind::D4,
            "no Pass the Pot in the list"
        );
    }

    #[test]
    fn apps_lists_the_games_then_settings() {
        let apps = draft().tipped(TipDir::Right);
        let names: std::vec::Vec<_> = (0..6).map(|i| apps.stepped(TipDir::Up, i).value()).collect();
        assert_eq!(names, ["Dice", "Pot", "Potato", "Pigs", "Settings", "Dice"]);
        assert_eq!(apps_on_settings().value().as_str(), "Settings");
    }

    #[test]
    fn choosing_a_game_swaps_the_ring() {
        let pot_mode = draft().tipped(TipDir::Right).tipped(TipDir::Up);
        assert_eq!(pot_mode.play, PlayMode::PassThePot);
        assert_eq!(pot_mode.page, Page::Apps);
        assert_eq!(pot_mode.ring(), &[Page::Apps, Page::Pot]);
        assert_eq!(pot_mode.tipped(TipDir::Left).page, Page::Pot);
        assert_eq!(pot_mode.tipped(TipDir::Down).play, PlayMode::Dice);
    }

    #[test]
    fn a_hold_on_settings_goes_into_its_pages_and_keeps_the_game() {
        let on = apps_on_settings();
        assert!(on.on_settings);
        assert_eq!(on.ring(), &[Page::Apps], "nowhere to wander while it's chosen");
        let s = settings_app();
        assert!(s.in_settings);
        assert_eq!(s.page, Page::Battery);
        assert_eq!(s.play, PlayMode::Dice, "the game in use, not the last one passed");
        assert_eq!(
            s.ring(),
            &[
                Page::Battery,
                Page::Setting(0),
                Page::Setting(1),
                Page::Setting(2),
                Page::Setting(3),
                Page::Setting(4),
                Page::Legal,
                Page::Power
            ]
        );
        assert_eq!(s.held(), Held::Save);
        // Settings opened from Pass the Pot saves back into Pass the Pot.
        let from_pot = hold(pot().tipped(TipDir::Right).stepped(TipDir::Down, 2));
        assert!(from_pot.in_settings);
        assert_eq!(from_pot.play, PlayMode::PassThePot);
    }

    #[test]
    fn each_setting_is_a_page_that_tips_through_its_values() {
        let brightness = settings_app().tipped(TipDir::Left);
        assert_eq!(brightness.page, Page::Setting(0));
        assert_eq!(brightness.page.title(), "Brightness");
        assert_eq!(brightness.value().as_str(), "70%");
        assert_eq!(brightness.tipped(TipDir::Up).value().as_str(), "100%");
        assert_eq!(brightness.stepped(TipDir::Up, 2).value().as_str(), "30%", "wraps");
        let haptics = brightness.tipped(TipDir::Left).tipped(TipDir::Up);
        assert_eq!(haptics.setting(HAPTICS), "Off");
        assert_eq!(haptics.setting(BRIGHTNESS), "70%", "each has its own choice");
    }

    #[test]
    fn a_hold_on_power_powers_off_and_taps_do_nothing_anywhere() {
        let power = settings_app().tipped(TipDir::Right);
        assert_eq!(power.page, Page::Power);
        assert!(power.powers_off());
        assert_eq!(power.held(), Held::PowerOff);
        assert!(!settings_app().powers_off());
        assert_eq!(power.view(50).caption, Some("Hold: off"));
    }

    #[test]
    fn battery_and_legal_are_to_read() {
        let battery = settings_app();
        assert_eq!(
            battery.view(78).value,
            Value::Text(String::try_from("78%").unwrap())
        );
        assert_eq!(battery.tipped(TipDir::Up), battery);
        let legal = battery.stepped(TipDir::Right, 2);
        assert_eq!(legal.page, Page::Legal);
        assert_eq!(legal.view(0).value, Value::Lines(["FCC ID", "TBD"]));
        assert_eq!(
            legal.tipped(TipDir::Up).view(0).value,
            Value::Lines(["IC", "TBD"])
        );
        assert_eq!(
            legal.tipped(TipDir::Down).view(0).value,
            Value::Lines(["Marks", "CE"])
        );
    }

    #[test]
    fn every_page_fits_the_held_screen_rules() {
        // Brief 3, 1.3: H2 lines at most 10 characters; values short.
        let mut drafts = std::vec![draft(), pot(), Draft::new(&pigs(6)), settings_app()];
        drafts.push(Draft::new(&pigs(2)).with_session(true));
        drafts.push(Draft::alone(&pigs(2), Page::Next));
        let mut seen = 0;
        for d in drafts {
            for i in 0..d.ring().len() {
                let page = d.stepped(TipDir::Left, i as i32);
                for v in 0..8 {
                    let page = page.stepped(TipDir::Up, v);
                    let view = page.view(100);
                    for line in [
                        view.status.as_str(),
                        view.title.as_str(),
                        view.caption.unwrap_or(""),
                    ] {
                        assert!(line.chars().count() <= 10, "{line:?} on {:?}", page.page);
                    }
                    match &view.value {
                        Value::Text(t) => assert!(t.chars().count() <= 4, "{t:?} on {:?}", page.page),
                        Value::Lines(lines) => {
                            for l in lines {
                                assert!(l.chars().count() <= 10, "{l:?}");
                            }
                        }
                        _ => {}
                    }
                    seen += 1;
                }
            }
        }
        assert!(seen > 100);
    }

    #[test]
    fn pass_the_pot_starts_with_three_bills_and_counts_one_to_three() {
        let d = pot();
        assert_eq!(d.page, Page::Pot, "opens on the game's own page");
        assert_eq!(d.page.title(), "Bills");
        assert_eq!(d.pot_count, 3, "everyone starts with three bills");
        assert_eq!(d.value().as_str(), "3");
        assert_eq!(d.tipped(TipDir::Down).pot_count, 2);
        assert_eq!(d.tipped(TipDir::Up).pot_count, 1, "wraps");
    }

    #[test]
    fn the_bills_in_hand_are_the_dice_rolled() {
        let s = Settings {
            play: PlayMode::PassThePot,
            pot_count: 2,
            ..Settings::default()
        };
        assert_eq!(s.active(), (DieKind::PassThePot, 2));
    }

    #[test]
    fn hot_potato_has_a_fuse_page() {
        let potato = draft().tipped(TipDir::Right).stepped(TipDir::Up, 2);
        assert_eq!(potato.play, PlayMode::HotPotato);
        assert_eq!(potato.setup(), Setup::HotPotato);
        assert_eq!(potato.setup().short_label().as_str(), "Potato");
        let fuse = potato.tipped(TipDir::Left);
        assert_eq!(fuse.page, Page::Fuse);
        assert_eq!(fuse.value().as_str(), "Mid");
        assert_eq!(fuse.tipped(TipDir::Up).fuse, Fuse::Long);
        assert_eq!(fuse.tipped(TipDir::Down).value().as_str(), "Fast");
        assert_eq!(fuse.tipped(TipDir::Left).page, Page::Apps);
    }

    #[test]
    fn fuse_ranges_grow() {
        for f in Fuse::ALL {
            let (lo, hi) = f.range_ms();
            assert!(lo < hi);
        }
        assert!(Fuse::Short.range_ms().1 <= Fuse::Medium.range_ms().1);
        assert!(Fuse::Medium.range_ms().1 <= Fuse::Long.range_ms().1);
    }

    #[test]
    fn switching_modes_keeps_the_dice_setup() {
        let mut s = Settings {
            die: DieKind::D6,
            count: 3,
            ..Settings::default()
        };
        let mut d = Draft::new(&s).tipped(TipDir::Right).tipped(TipDir::Up);
        d = d.tipped(TipDir::Left).tipped(TipDir::Down);
        d.commit(&mut s);
        assert_eq!(s.active(), (DieKind::PassThePot, 2));
        assert_eq!((s.die, s.count), (DieKind::D6, 3));
        let d = Draft::new(&s);
        assert_eq!(d.page, Page::Pot);
        let d = d.tipped(TipDir::Right).tipped(TipDir::Down);
        d.commit(&mut s);
        assert_eq!(s.active(), (DieKind::D6, 3));
        assert_eq!(s.pot_count, 2);
    }

    #[test]
    fn a_die_with_only_dice_still_has_apps_for_settings() {
        let s = Settings {
            enabled: ModeSet::DICE,
            play: PlayMode::PassThePot,
            ..Settings::default()
        };
        assert_eq!(s.active(), (DieKind::D20, 1), "always dice");
        let apps = Draft::new(&s).tipped(TipDir::Right);
        assert_eq!(apps.page, Page::Apps);
        assert_eq!(apps.value().as_str(), "Dice");
        assert_eq!(apps.tipped(TipDir::Up).value().as_str(), "Settings");
        assert_eq!(apps.stepped(TipDir::Up, 2).value().as_str(), "Dice");
    }

    #[test]
    fn apps_offers_only_licensed_games_that_are_turned_on() {
        let s = Settings {
            licensed: ModeSet::DICE.with(ModeId::HotPotato).with(ModeId::PigToss),
            enabled: ModeSet::ALL.without(ModeId::PigToss),
            ..Settings::default()
        };
        assert_eq!(s.modes(), ModeSet::DICE.with(ModeId::HotPotato));
        let apps = Draft::new(&s).tipped(TipDir::Right);
        assert_eq!(apps.tipped(TipDir::Up).play, PlayMode::HotPotato);
        assert!(apps.stepped(TipDir::Up, 2).on_settings);
        assert_eq!(apps.stepped(TipDir::Up, 3).play, PlayMode::Dice);
    }

    #[test]
    fn dice_is_always_on_and_a_mode_turned_off_falls_back_to_it() {
        let mut s = Settings {
            enabled: ModeSet::EMPTY,
            play: PlayMode::PigToss,
            ..Settings::default()
        };
        assert_eq!(s.modes(), ModeSet::DICE);
        assert_eq!(s.play(), PlayMode::Dice);
        s.enabled = ModeSet::DICE.with(ModeId::PigToss);
        assert_eq!(s.play(), PlayMode::PigToss, "the saved mode comes back");
        s.licensed = ModeSet::DICE;
        assert_eq!(s.play(), PlayMode::Dice, "not without a license");
    }

    #[test]
    fn play_modes_map_to_their_wire_ids() {
        for m in PlayMode::ALL {
            assert_eq!(PlayMode::from_id(m.id()), m);
        }
        assert_eq!(PlayMode::ALL.map(PlayMode::id), ModeId::ALL);
    }

    #[test]
    fn nothing_changes_until_committed() {
        let s = Settings::default();
        let d = Draft::new(&s).tipped(TipDir::Right).tipped(TipDir::Up);
        assert_eq!(d.active(), (DieKind::PassThePot, 3), "three bills to start");
        assert_eq!(s.active(), (DieKind::D20, 1));
    }

    #[test]
    fn several_steps_at_once() {
        let d = draft();
        assert_eq!(d.stepped(TipDir::Left, 2).page, Page::Apps);
        assert_eq!(d.stepped(TipDir::Up, 3).count, 4);
        assert_eq!(d.stepped(TipDir::Up, -2).count, 9);
        assert_eq!(d.stepped(TipDir::Up, 0), d);
    }

    #[test]
    fn short_labels() {
        assert_eq!(short_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(short_label(DieKind::PassThePot, 1).as_str(), "Pot ×1");
    }

    #[test]
    fn smoke_and_sleep_settings_map_to_values() {
        let mut s = Settings::default();
        assert_eq!(s.smoke_amount(), Amount::Full);
        assert_eq!(s.sleep_after_ms(), Some(120_000));
        let sugar = settings_app().stepped(TipDir::Left, 3);
        assert_eq!(sugar.page.title(), "Sugar");
        sugar.tipped(TipDir::Up).commit(&mut s);
        assert_eq!(s.smoke_amount(), Amount::Off, "Full wraps to Off");
        sugar.stepped(TipDir::Up, 2).commit(&mut s);
        assert_eq!(s.smoke_amount(), Amount::Light);
        let sleep = sugar.tipped(TipDir::Left);
        assert_eq!(sleep.page.title(), "Sleep");
        for (k, expect) in [
            (1, Some(300_000)),
            (2, None),
            (3, Some(30_000)),
            (4, Some(60_000)),
        ] {
            sleep.stepped(TipDir::Up, k).commit(&mut s);
            assert_eq!(s.sleep_after_ms(), expect);
        }
    }

    #[test]
    fn the_settings_are_the_ones_that_do_something() {
        let names: [&str; 5] = core::array::from_fn(|i| SETTINGS[i].name);
        assert_eq!(names, ["Brightness", "Haptics", "Sugar", "Sleep", "Bluetooth"]);
        assert_eq!(Settings::default().choices[HAPTICS], 1, "haptics start on");
    }

    #[test]
    fn saving_keeps_the_choices_and_defaults_match_the_mockup() {
        let mut s = Settings::default();
        assert_eq!(s.brightness_pct(), 70);
        assert!(s.haptics_on() && s.bluetooth_on());
        let d = settings_app()
            .tipped(TipDir::Left)
            .tipped(TipDir::Up)
            .tipped(TipDir::Left)
            .tipped(TipDir::Up);
        d.commit(&mut s);
        assert_eq!(s.brightness_pct(), 100);
        assert!(!s.haptics_on());
    }

    #[test]
    fn the_short_id_uses_every_byte_of_the_serial() {
        let a = [0x01, 0x23, 0x5B, 0x0E, 0, 0, 0, 0, 0xEE];
        assert_eq!(short_id(&a), short_id(&a));
        for i in 0..a.len() {
            let mut b = a;
            b[i] ^= 1;
            assert_ne!(short_id(&a), short_id(&b), "byte {i}");
        }
    }

    #[test]
    fn choosing_pig_toss_goes_to_players_then_each_players_token() {
        let pigs = draft().tipped(TipDir::Right).stepped(TipDir::Up, 3);
        assert_eq!(pigs.play, PlayMode::PigToss);
        let players = hold(pigs);
        assert_eq!(players.page, Page::Players);
        let players = players.tipped(TipDir::Up);
        assert_eq!(players.players, 3);
        let p1 = hold(players);
        assert_eq!(p1.page, Page::Token(0));
        assert_eq!(p1.view(0).title.as_str(), "Player 1");
        assert_eq!(p1.value().as_str(), "A");
        let p1 = p1.stepped(TipDir::Up, 9);
        assert_eq!(p1.value().as_str(), "J");
        let p2 = hold(p1);
        let p2 = p2.tipped(TipDir::Down).tipped(TipDir::Down);
        assert_eq!(p2.value().as_str(), "Star", "B, then A, then the symbols");
        let p3 = hold(p2);
        assert_eq!(p3.page, Page::Token(2));
        assert_eq!(p3.held(), Held::Save, "the last player saves");
        assert!(p3.set_up() && !p3.ended);
        let mut s = Settings::default();
        p3.commit(&mut s);
        assert_eq!(s.play(), PlayMode::PigToss);
        assert_eq!(s.players, 3);
        assert_eq!(s.tokens[0].initial(), Some('J'));
        assert_eq!(s.tokens[1].as_symbol(), Some(crate::pigs::Symbol::Star));
    }

    #[test]
    fn setting_up_tips_through_players_and_tokens_only() {
        let p1 = hold(Draft::new(&pigs(2)));
        assert_eq!(p1.page, Page::Token(0));
        assert_eq!(p1.ring(), &[Page::Players, Page::Token(0), Page::Token(1)]);
        assert_eq!(p1.stepped(TipDir::Left, 2).page, Page::Players, "wraps");
    }

    #[test]
    fn without_a_game_pig_toss_cant_be_saved_until_one_is_set_up() {
        let d = Draft::new(&pigs(2));
        assert_eq!(d.page, Page::Players);
        assert_eq!(hold(d).page, Page::Token(0));
        // From Apps a hold goes to Players first.
        let apps = d.tipped(TipDir::Left);
        assert_eq!(apps.page, Page::Apps);
        assert_eq!(hold(apps).page, Page::Players);
    }

    #[test]
    fn a_game_in_play_has_end_game_instead_of_players() {
        let d = Draft::new(&pigs(2)).with_session(true);
        assert_eq!(d.page, Page::Apps, "opens a tip away from End game");
        assert_eq!(d.ring(), &[Page::Apps, Page::EndGame]);
        assert_eq!(d.held(), Held::Save, "the menu doesn't end it");
        let end = d.tipped(TipDir::Left);
        assert_eq!(end.page, Page::EndGame);
        assert_eq!(end.tipped(TipDir::Up), end, "nothing to tip");
        assert_eq!(end.view(0).caption, Some("Clears all"));
    }

    #[test]
    fn ending_the_game_brings_players_back() {
        let end = Draft::new(&pigs(2)).with_session(true).tipped(TipDir::Left);
        let players = hold(end);
        assert_eq!(players.page, Page::Players);
        assert!(players.ended && !players.live);
        let p2 = hold(hold(players));
        assert_eq!(p2.page, Page::Token(1));
        assert!(p2.ended && p2.set_up());
    }

    #[test]
    fn switching_modes_keeps_the_game() {
        let d = Draft::new(&pigs(2)).with_session(true);
        let dice = d.stepped(TipDir::Up, 2);
        assert_eq!(dice.play, PlayMode::Dice);
        assert_eq!(dice.held(), Held::Save);
        assert!(!dice.ended);
    }

    #[test]
    fn a_game_opens_one_page_alone() {
        let s = Settings {
            play: PlayMode::PassThePot,
            ..Settings::default()
        };
        let d = Draft::alone(&s, Page::Pot);
        assert_eq!(d.ring(), &[Page::Pot]);
        assert_eq!(d.tipped(TipDir::Left).page, Page::Pot, "nowhere else to go");
        assert_eq!(d.tipped(TipDir::Down).pot_count, 2);
        assert_eq!(d.held(), Held::Save);
    }

    #[test]
    fn after_a_win_a_hold_rematches_or_sets_up_players_again() {
        let d = Draft::alone(&pigs(3), Page::Next).with_session(true);
        assert_eq!(d.ring(), &[Page::Next]);
        assert_eq!(d.value().as_str(), "Redo");
        assert_eq!(d.view(0).caption, Some("Rematch"));
        assert_eq!(d.held(), Held::Save);
        assert!(d.restart());
        let players = d.tipped(TipDir::Up);
        assert_eq!(players.value().as_str(), "New");
        assert_eq!(players.view(0).caption, Some("Players"));
        let setup = hold(players);
        assert_eq!(setup.page, Page::Players);
        assert_eq!(setup.ring().len(), 1 + 3, "Players and each player's token");
        assert!(!setup.restart());
        assert!(setup.set_up(), "saving at the end starts the new game");
    }
}
