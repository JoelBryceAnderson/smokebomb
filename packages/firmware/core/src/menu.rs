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

/// The Settings page's items, as the mockup shows them (the defaults are its
/// values). Owner and About are display-only; the phone sets the owner.
pub const SETTINGS: [Item; 11] = [
    Item::choice("Brightness", &["30%", "50%", "70%", "100%"], 2),
    Item::choice("Haptics", &["Off", "Light", "Strong"], 2),
    Item::choice("Smoke", &["Off", "Light", "Full"], 2),
    Item::choice("Large text", &["Off", "On"], 0),
    Item::choice("Sleep after", &["30 s", "1 min", "2 min", "5 min", "Never"], 2),
    Item::choice("Night mode", &["Off", "Auto", "On"], 1),
    Item::choice("Bluetooth", &["Off", "On"], 1),
    Item::choice("Verified rolls", &["Off", "On"], 0),
    Item::fixed("Owner", &["Joel"]),
    Item::fixed("Power off", &["Tap to power off"]),
    Item::fixed("About", &["v0.1.0 · SB-0042"]),
];

const BRIGHTNESS: usize = 0;
const HAPTICS: usize = 1;
const BLUETOOTH: usize = 6;
const POWER_OFF: u8 = 9;

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
}

impl PlayMode {
    /// Menu order on the Mode page.
    pub const ALL: [PlayMode; 3] = [PlayMode::Dice, PlayMode::PassThePot, PlayMode::HotPotato];

    pub const fn name(self) -> &'static str {
        match self {
            PlayMode::Dice => "Dice",
            PlayMode::PassThePot => "Pass the Pot",
            PlayMode::HotPotato => "Hot Potato",
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
    /// Pass the Pot's option: how many pots to pass.
    Pot,
    /// Hot Potato's option: how long the fuse may run.
    Fuse,
    Settings,
}

impl Page {
    pub const fn title(self) -> &'static str {
        match self {
            Page::Mode => "Mode",
            Page::Count => "How many dice",
            Page::Die => "Which die",
            Page::Pot => "How many pots",
            Page::Fuse => "Fuse length",
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
        }
    }

    /// The line under the label on the success screen.
    pub const fn nudge(self) -> &'static str {
        match self {
            Setup::Roll(..) => "Ready to roll",
            Setup::HotPotato => "Shake to light",
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
    pub pot_count: u8,
    pub fuse: Fuse,
    /// The chosen option of each [`SETTINGS`] item.
    pub choices: Choices,
}

impl Default for Settings {
    fn default() -> Self {
        // The mockup's default setup: a single d20.
        Self {
            modes: true,
            play: PlayMode::Dice,
            die: DieKind::D20,
            count: 1,
            pot_count: 1,
            fuse: Fuse::Medium,
            choices: default_choices(),
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
            PlayMode::Dice | PlayMode::HotPotato => (self.die, self.count),
            PlayMode::PassThePot => (DieKind::PassThePot, self.pot_count),
        }
    }

    /// What the die tells the person it's set up for.
    pub fn setup(&self) -> Setup {
        match self.play() {
            PlayMode::HotPotato => Setup::HotPotato,
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
    /// Index into [`SETTINGS`].
    pub setting: u8,
    pub choices: Choices,
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
            setting: 0,
            choices: s.choices,
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

    pub fn commit(&self, s: &mut Settings) {
        s.play = self.play;
        s.die = self.die;
        s.count = self.count;
        s.pot_count = self.pot_count;
        s.fuse = self.fuse;
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
    fn pass_the_pot_counts_one_to_three() {
        let d = pot();
        assert_eq!(d.page, Page::Pot, "opens on the game's own page");
        assert_eq!(d.pot_count, 1);
        assert_eq!(d.tipped(TipDir::Up).pot_count, 2);
        assert_eq!(d.tipped(TipDir::Down).pot_count, 3);
        assert_eq!(d.tipped(TipDir::Down).tipped(TipDir::Up).pot_count, 1);
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
        // Into Pass the Pot with two pots, then saved.
        let mut d = Draft::new(&s).tipped(TipDir::Right).tipped(TipDir::Up);
        d = d.tipped(TipDir::Left).tipped(TipDir::Up);
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
        assert_eq!(d.active(), (DieKind::PassThePot, 1));
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
    fn power_off_is_the_tenth_setting() {
        let mut d = settings_page();
        for _ in 0..9 {
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
        for name in ["Owner", "Power off", "About"] {
            while d.setting().0 != name {
                d = d.tipped(TipDir::Up);
            }
            assert_eq!(d.tapped(), d, "{name}");
        }
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
