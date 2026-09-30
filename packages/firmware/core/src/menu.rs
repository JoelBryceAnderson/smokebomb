//! On-die setup menu (SIM_SPEC C3).
//!
//! Hold any screen to open the menu there. Tipping the die moves through it:
//! left and right turn the page, up and down change the value (on the
//! Settings page, up and down pick the item). A tap changes the selected
//! Settings item, or powers the die off on Power off. Everything happens on
//! a [`Draft`]; a hold always saves it and returns to the roll, and a throw,
//! docking or 25 s without input leaves the setup as it was.

use core::fmt::Write as _;

use heapless::String;
use smokebomb_shared::types::{MAX_DICE, MAX_POT_DICE};
use smokebomb_shared::DieKind;

use crate::smoke::Amount;
use crate::tips::TipDir;

/// One row of the Settings page. A tap steps `options`; a row without any is
/// display-only (or, for [`POWER_OFF`], an action).
pub struct Item {
    pub name: &'static str,
    /// The values a tap cycles through, or the single text a fixed row shows.
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

    const fn fixed(name: &'static str, text: &'static [&'static str]) -> Self {
        Self::choice(name, text, 0)
    }

    /// Does a tap change this row?
    pub const fn editable(&self) -> bool {
        self.options.len() > 1
    }
}

/// The Settings page's items. Owner, Power off, About and Regulatory are fixed
/// rows; the phone will set the owner, About shows the real version and the
/// die's id (see [`Draft::setting_value`]), and Regulatory shows the approval
/// numbers (placeholders until the die is certified). (The mockup had three more: Large text,
/// Night mode and Verified rolls. See SIM_SPEC H13.)
pub const SETTINGS: [Item; 9] = [
    Item::choice("Brightness", &["30%", "50%", "70%", "100%"], 2),
    Item::choice("Haptics", &["Off", "On"], 1),
    Item::choice("Smoke", &["Off", "Light", "Full"], 2),
    Item::choice("Sleep after", &["30 s", "1 min", "2 min", "5 min", "Never"], 2),
    Item::choice("Bluetooth", &["Off", "On"], 1),
    Item::fixed("Owner", &["Joel"]),
    Item::fixed("Power off", &["Tap to power off"]),
    Item::fixed("About", &[""]),
    Item::fixed("Regulatory", &[""]),
];

const BRIGHTNESS: usize = 0;
const HAPTICS: usize = 1;
const SMOKE: usize = 2;
const SLEEP: usize = 3;
const BLUETOOTH: usize = 4;
const POWER_OFF: u8 = 6;
const ABOUT: u8 = 7;
const REGULATORY: u8 = 8;

/// What the Regulatory row shows, on two lines. FCC ID and IC are
/// placeholders until the die is certified.
const REGULATORY_LINES: [&str; 2] = ["FCC ID: TBD", "IC: TBD · CE · SC-1"];

/// The chosen option of every Settings item.
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
    /// Menu order on the Mode page.
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

    /// Whether a throw rolls and signs dice in this mode.
    pub const fn rolls(self) -> bool {
        !matches!(self, PlayMode::HotPotato)
    }

    /// The menu's pages in this mode. Tipping left goes to the next one, so
    /// Mode is one tip right of the first page. With `modes` off there is no
    /// Mode page and the menu is the plain dice menu.
    pub const fn ring(self, modes: bool) -> &'static [Page] {
        match (self, modes) {
            (PlayMode::Dice, true) => &[Page::Mode, Page::Count, Page::Die, Page::Settings],
            (PlayMode::PassThePot, true) => &[Page::Mode, Page::Pot, Page::Settings],
            (PlayMode::HotPotato, true) => &[Page::Mode, Page::Fuse, Page::Settings],
            (PlayMode::PigToss, true) => &[Page::Mode, Page::Players, Page::Settings],
            _ => &[Page::Count, Page::Die, Page::Settings],
        }
    }

    /// The page the menu opens on: the mode's first page after Mode.
    pub const fn home(self, modes: bool) -> Page {
        self.ring(modes)[if modes { 1 } else { 0 }]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Mode,
    Count,
    Die,
    /// Pass the Pot's option: how many bills you hold, which is how many
    /// dice you roll (up to three).
    Pot,
    /// Hot Potato's option: how long the fuse may run.
    Fuse,
    /// Pig Toss' option: how many people are playing.
    Players,
    Settings,
}

impl Page {
    pub const fn title(self) -> &'static str {
        match self {
            Page::Mode => "Mode",
            Page::Count => "How many dice",
            Page::Die => "Which die",
            Page::Pot => "Bills in hand",
            Page::Fuse => "Fuse length",
            Page::Players => "Players",
            Page::Settings => "Settings",
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
            Fuse::Short => "Short",
            Fuse::Medium => "Medium",
            Fuse::Long => "Long",
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

    /// The line under the label on the success screen.
    pub const fn nudge(self) -> &'static str {
        match self {
            Setup::Roll(..) => "Ready to roll",
            Setup::HotPotato => "Shake to light",
            Setup::Pigs(_) => "Shake to roll",
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
    /// Whether the menu has a Mode page. Off, the die is always in Dice
    /// mode and the menu is Count, Die, Settings.
    pub modes: bool,
    pub play: PlayMode,
    pub die: DieKind,
    pub count: u8,
    /// Pass the Pot: the bills in your hand, which is how many dice you
    /// roll. Everyone starts with three.
    pub pot_count: u8,
    pub fuse: Fuse,
    /// Pig Toss: how many people are playing.
    pub players: u8,
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
            modes: true,
            play: PlayMode::Dice,
            die: DieKind::D20,
            count: 1,
            pot_count: 3,
            fuse: Fuse::Medium,
            players: crate::pigs::MIN_PLAYERS,
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

    /// The mode in use: always Dice when the menu has no Mode page.
    pub fn play(&self) -> PlayMode {
        if self.modes {
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
    pub modes: bool,
    pub play: PlayMode,
    pub die: DieKind,
    pub count: u8,
    pub pot_count: u8,
    pub fuse: Fuse,
    pub players: u8,
    /// Index into [`SETTINGS`].
    pub setting: u8,
    pub choices: Choices,
    pub device_id: u16,
}

impl Draft {
    /// The menu opens on the current mode's first page.
    pub fn new(s: &Settings) -> Self {
        Self {
            page: s.play().home(s.modes),
            modes: s.modes,
            play: s.play(),
            die: s.die,
            count: s.count,
            pot_count: s.pot_count,
            fuse: s.fuse,
            players: s.players,
            setting: 0,
            choices: s.choices,
            device_id: s.device_id,
        }
    }

    /// The pages the draft can tip through.
    pub fn ring(&self) -> &'static [Page] {
        self.play.ring(self.modes)
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
                    Page::Mode => {
                        let i = PlayMode::ALL.iter().position(|m| *m == self.play).unwrap_or(0);
                        next.play = PlayMode::ALL[step(i, by, PlayMode::ALL.len())];
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
                    Page::Settings => {
                        next.setting = step(self.setting as usize, by, SETTINGS.len()) as u8;
                    }
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

    /// A tap on this draft powers the die off.
    pub fn power_off_selected(&self) -> bool {
        self.page == Page::Settings && self.setting == POWER_OFF
    }

    /// The draft after a tap: the selected Settings item moves to its next
    /// option, wrapping. Taps do nothing on the other pages or on a fixed row.
    pub fn tapped(self) -> Self {
        let item = &SETTINGS[self.setting as usize];
        if self.page != Page::Settings || !item.editable() {
            return self;
        }
        let mut next = self;
        let c = &mut next.choices[self.setting as usize];
        *c = step(*c as usize, 1, item.options.len()) as u8;
        next
    }

    /// The selected Settings item's name and current value.
    pub fn setting(&self) -> (&'static str, &'static str) {
        let item = &SETTINGS[self.setting as usize];
        let i = self.choices[self.setting as usize] as usize;
        (item.name, item.options[i.min(item.options.len() - 1)])
    }

    /// The text the Settings page shows for the selected item: its value, or
    /// for About the firmware version and the die's id.
    pub fn setting_value(&self) -> String<24> {
        let mut s = String::new();
        if self.setting == REGULATORY {
            let _ = s.push_str(REGULATORY_LINES[0]);
        } else if self.setting == ABOUT {
            let _ = write!(s, "v{} · SC-{:04X}", env!("CARGO_PKG_VERSION"), self.device_id);
        } else {
            let _ = s.push_str(self.setting().1);
        }
        s
    }

    /// A second line under the selected item's value (Regulatory only).
    pub fn setting_detail(&self) -> Option<&'static str> {
        (self.setting == REGULATORY).then_some(REGULATORY_LINES[1])
    }

    pub fn commit(&self, s: &mut Settings) {
        s.play = self.play;
        s.die = self.die;
        s.count = self.count;
        s.pot_count = self.pot_count;
        s.fuse = self.fuse;
        s.players = self.players;
        s.choices = self.choices;
    }

    /// The setup the draft would save.
    pub fn setup(&self) -> Setup {
        let mut s = Settings {
            modes: self.modes,
            ..Settings::default()
        };
        self.commit(&mut s);
        s.setup()
    }

    /// The die and count the draft would roll.
    pub fn active(&self) -> (DieKind, u8) {
        let mut s = Settings {
            modes: self.modes,
            ..Settings::default()
        };
        self.commit(&mut s);
        s.active()
    }

    /// The page's big value (not used on the Settings page).
    pub fn value(&self) -> String<16> {
        let mut s = String::new();
        let _ = match self.page {
            Page::Mode => write!(s, "{}", self.play.name()),
            Page::Count => write!(s, "{}", self.count),
            Page::Die => write!(s, "{}", self.die.wire_name()),
            Page::Pot => write!(s, "{}", self.pot_count),
            Page::Fuse => write!(s, "{}", self.fuse.name()),
            Page::Players => write!(s, "{}", self.players),
            Page::Settings => write!(s, "{}", self.setting().0),
        };
        s
    }
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

    #[test]
    fn dice_mode_opens_on_the_count_as_before() {
        let d = draft();
        assert_eq!(d.page, Page::Count);
        assert_eq!(d.tipped(TipDir::Up).count, 2);
        assert_eq!(d.tipped(TipDir::Left).page, Page::Die);
    }

    #[test]
    fn mode_is_one_tip_right_of_the_count() {
        let d = draft();
        assert_eq!(d.tipped(TipDir::Right).page, Page::Mode);
        assert_eq!(d.tipped(TipDir::Left).tipped(TipDir::Left).page, Page::Settings);
        assert_eq!(
            d.tipped(TipDir::Left)
                .tipped(TipDir::Left)
                .tipped(TipDir::Left)
                .page,
            Page::Mode
        );
        let full_circle = (0..4).fold(d, |d, _| d.tipped(TipDir::Left));
        assert_eq!(full_circle, d);
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
        let d4 = Draft {
            die: DieKind::D4,
            ..die
        };
        assert_eq!(d4.tipped(TipDir::Down).die, DieKind::D100);
    }

    #[test]
    fn choosing_a_mode_swaps_the_ring() {
        let mode = draft().tipped(TipDir::Right);
        let pot_mode = mode.tipped(TipDir::Up);
        assert_eq!(pot_mode.play, PlayMode::PassThePot);
        assert_eq!(pot_mode.page, Page::Mode);
        assert_eq!(pot_mode.page_index(), 0);
        assert_eq!(pot_mode.tipped(TipDir::Left).page, Page::Pot);
        assert_eq!(
            pot_mode.tipped(TipDir::Left).tipped(TipDir::Left).page,
            Page::Settings
        );
        assert_eq!(pot_mode.tipped(TipDir::Down).play, PlayMode::Dice);
    }

    #[test]
    fn pass_the_pot_starts_with_three_bills_and_counts_one_to_three() {
        let d = pot();
        assert_eq!(d.page, Page::Pot, "opens on the game's own page");
        assert_eq!(d.page.title(), "Bills in hand");
        assert_eq!(d.pot_count, 3, "everyone starts with three bills");
        assert_eq!(d.value().as_str(), "3");
        assert_eq!(d.tipped(TipDir::Down).pot_count, 2);
        assert_eq!(d.tipped(TipDir::Down).tipped(TipDir::Down).pot_count, 1);
        assert_eq!(d.tipped(TipDir::Up).pot_count, 1, "wraps");
        assert_eq!(d.tipped(TipDir::Down).tipped(TipDir::Up).pot_count, 3);
    }

    #[test]
    fn the_bills_in_hand_are_the_dice_rolled() {
        let s = Settings {
            play: PlayMode::PassThePot,
            pot_count: 2,
            ..Settings::default()
        };
        assert_eq!(s.active(), (DieKind::PassThePot, 2));
        assert_eq!(Settings::default().pot_count, 3);
    }

    #[test]
    fn hot_potato_has_a_fuse_page_and_does_not_roll() {
        let mode = draft().tipped(TipDir::Right);
        let potato = mode.tipped(TipDir::Up).tipped(TipDir::Up);
        assert_eq!(potato.play, PlayMode::HotPotato);
        assert!(!potato.play.rolls());
        assert_eq!(potato.setup(), Setup::HotPotato);
        assert_eq!(potato.setup().label().as_str(), "Hot Potato");
        assert_eq!(potato.setup().short_label().as_str(), "Potato");
        let fuse = potato.tipped(TipDir::Left);
        assert_eq!(fuse.page, Page::Fuse);
        assert_eq!(fuse.fuse, Fuse::Medium);
        assert_eq!(fuse.tipped(TipDir::Up).fuse, Fuse::Long);
        assert_eq!(fuse.tipped(TipDir::Up).tipped(TipDir::Up).fuse, Fuse::Short);
        assert_eq!(fuse.tipped(TipDir::Down).value().as_str(), "Short");
        assert_eq!(fuse.tipped(TipDir::Left).page, Page::Settings);
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
        // Into Pass the Pot, down from three bills to two, then saved.
        let mut d = Draft::new(&s).tipped(TipDir::Right).tipped(TipDir::Up);
        d = d.tipped(TipDir::Left).tipped(TipDir::Down);
        d.commit(&mut s);
        assert_eq!(s.active(), (DieKind::PassThePot, 2));
        assert_eq!((s.die, s.count), (DieKind::D6, 3));
        // And back to Dice.
        let d = Draft::new(&s);
        assert_eq!(d.page, Page::Pot);
        let d = d.tipped(TipDir::Right).tipped(TipDir::Down);
        d.commit(&mut s);
        assert_eq!(s.active(), (DieKind::D6, 3));
        assert_eq!(s.pot_count, 2);
    }

    #[test]
    fn without_modes_the_menu_is_the_plain_dice_menu() {
        let s = Settings {
            modes: false,
            play: PlayMode::PassThePot,
            ..Settings::default()
        };
        assert_eq!(s.active(), (DieKind::D20, 1), "always dice");
        let d = Draft::new(&s);
        assert_eq!(d.page, Page::Count);
        assert_eq!(d.tipped(TipDir::Right).page, Page::Settings);
        assert_eq!(d.tipped(TipDir::Left).page, Page::Die);
        assert_eq!(d.ring().len(), 3);
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
        assert_eq!(d.stepped(TipDir::Left, 2).page, Page::Settings);
        assert_eq!(d.stepped(TipDir::Up, 3).count, 4);
        assert_eq!(d.stepped(TipDir::Up, -2).count, 9);
        assert_eq!(d.stepped(TipDir::Up, 0), d);
    }

    #[test]
    fn short_labels() {
        assert_eq!(short_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(short_label(DieKind::PassThePot, 1).as_str(), "Pot ×1");
    }

    /// The Settings page, on Brightness, from the default menu.
    fn settings_page() -> Draft {
        draft().tipped(TipDir::Left).tipped(TipDir::Left)
    }

    #[test]
    fn power_off_is_the_seventh_setting() {
        let mut d = settings_page();
        for _ in 0..6 {
            assert!(!d.power_off_selected());
            d = d.tipped(TipDir::Up);
        }
        assert_eq!(d.setting().0, "Power off");
        assert!(d.power_off_selected());
    }

    #[test]
    fn a_tap_steps_the_selected_setting_and_wraps() {
        let mut d = settings_page();
        assert_eq!(d.setting(), ("Brightness", "70%"));
        d = d.tapped();
        assert_eq!(d.setting(), ("Brightness", "100%"));
        d = d.tapped();
        assert_eq!(d.setting(), ("Brightness", "30%"));
        // Each item has its own choice.
        let d = d.tipped(TipDir::Up).tapped();
        assert_eq!(d.setting(), ("Haptics", "Off"));
        assert_eq!(d.tipped(TipDir::Down).setting(), ("Brightness", "30%"));
    }

    #[test]
    fn a_tap_does_nothing_off_the_settings_page_or_on_fixed_rows() {
        let d = draft();
        assert_eq!(d.tapped(), d);
        assert_eq!(d.tipped(TipDir::Left).tapped(), d.tipped(TipDir::Left));
        let mut d = settings_page();
        for name in ["Owner", "Power off", "About", "Regulatory"] {
            while d.setting().0 != name {
                d = d.tipped(TipDir::Up);
            }
            assert_eq!(d.tapped(), d, "{name}");
        }
    }

    #[test]
    fn smoke_and_sleep_settings_map_to_values() {
        let mut s = Settings::default();
        assert_eq!(s.smoke_amount(), Amount::Full);
        assert_eq!(s.sleep_after_ms(), Some(120_000));
        // Smoke is the third item, Sleep after the fourth.
        let mut d = settings_page().tipped(TipDir::Up).tipped(TipDir::Up);
        assert_eq!(d.setting().0, "Smoke");
        d = d.tapped();
        d.commit(&mut s);
        assert_eq!(s.smoke_amount(), Amount::Off, "Full wraps to Off");
        d = d.tapped();
        d.commit(&mut s);
        assert_eq!(s.smoke_amount(), Amount::Light);
        d = d.tipped(TipDir::Up);
        assert_eq!(d.setting().0, "Sleep after");
        for expect in [Some(300_000), None, Some(30_000), Some(60_000)] {
            d = d.tapped();
            d.commit(&mut s);
            assert_eq!(s.sleep_after_ms(), expect);
        }
    }

    #[test]
    fn the_settings_are_the_ones_that_do_something() {
        let names: [&str; 9] = core::array::from_fn(|i| SETTINGS[i].name);
        assert_eq!(
            names,
            [
                "Brightness",
                "Haptics",
                "Smoke",
                "Sleep after",
                "Bluetooth",
                "Owner",
                "Power off",
                "About",
                "Regulatory"
            ]
        );
        assert_eq!(Settings::default().choices[HAPTICS], 1, "haptics start on");
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
    fn about_shows_the_version_and_the_dies_id() {
        let s = Settings {
            device_id: 0xA1B2,
            ..Settings::default()
        };
        let mut d = Draft::new(&s).tipped(TipDir::Left).tipped(TipDir::Left);
        d = d.tipped(TipDir::Down).tipped(TipDir::Down); // wraps to Regulatory, then About
        assert_eq!(d.setting().0, "About");
        assert_eq!(
            d.setting_value().as_str(),
            concat!("v", env!("CARGO_PKG_VERSION"), " · SC-A1B2")
        );
        // Other rows show their value.
        assert_eq!(
            d.tipped(TipDir::Down).setting_value().as_str(),
            "Tap to power off"
        );
    }

    #[test]
    fn regulatory_follows_about_with_placeholder_numbers() {
        let d = settings_page().tipped(TipDir::Down);
        assert_eq!(d.setting().0, "Regulatory");
        assert_eq!(d.setting_value().as_str(), "FCC ID: TBD");
        assert_eq!(d.setting_detail(), Some("IC: TBD · CE · SC-1"));
        assert_eq!(d.tipped(TipDir::Up).setting_detail(), None);
    }

    #[test]
    fn saving_keeps_the_choices_and_defaults_match_the_mockup() {
        let mut s = Settings::default();
        assert_eq!(s.brightness_pct(), 70);
        assert!(s.haptics_on() && s.bluetooth_on());
        let d = settings_page().tapped().tipped(TipDir::Up).tapped();
        d.commit(&mut s);
        assert_eq!(s.brightness_pct(), 100);
        assert!(!s.haptics_on());
    }
}
