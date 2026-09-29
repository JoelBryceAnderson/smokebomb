//! On-die setup menu (SIM_SPEC C3).
//!
//! Hold any screen to open the menu there. Tipping the die moves through it:
//! left and right turn the page, up and down change the value. Everything
//! happens on a [`Draft`]; holding again saves it, and a throw, docking or
//! 25 s without input leaves the setup as it was.

use core::fmt::Write as _;

use heapless::String;
use smokebomb_shared::types::{MAX_DICE, MAX_POT_DICE};
use smokebomb_shared::DieKind;

use crate::tips::TipDir;

/// The Settings page's items, as the mockup shows them. They are
/// display-only for now: tipping up and down moves through them, and
/// holding on Restart restarts the die.
pub const SETTINGS: [(&str, &str); 11] = [
    ("Brightness", "70%"),
    ("Haptics", "Strong"),
    ("Smoke", "Full"),
    ("Large text", "Off"),
    ("Sleep after", "2 min"),
    ("Night mode", "Auto"),
    ("Bluetooth", "On"),
    ("Verified rolls", "Off"),
    ("Owner", "Joel"),
    ("Restart", "Hold to restart"),
    ("About", "v0.1.0 · SB-0042"),
];

const RESTART: u8 = 9;

/// What the die is being used for. Dice keeps the count and die pages;
/// each game brings its own options page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    Dice,
    PassThePot,
}

impl PlayMode {
    /// Menu order on the Mode page.
    pub const ALL: [PlayMode; 2] = [PlayMode::Dice, PlayMode::PassThePot];

    pub const fn name(self) -> &'static str {
        match self {
            PlayMode::Dice => "Dice",
            PlayMode::PassThePot => "Pass the Pot",
        }
    }

    /// The menu's pages in this mode. Tipping left goes to the next one, so
    /// Mode is one tip right of the first page. With `modes` off there is no
    /// Mode page and the menu is the plain dice menu.
    pub const fn ring(self, modes: bool) -> &'static [Page] {
        match (self, modes) {
            (PlayMode::Dice, true) => &[Page::Mode, Page::Count, Page::Die, Page::Settings],
            (PlayMode::PassThePot, true) => &[Page::Mode, Page::Pot, Page::Settings],
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
    Settings,
}

impl Page {
    pub const fn title(self) -> &'static str {
        match self {
            Page::Mode => "Mode",
            Page::Count => "How many dice",
            Page::Die => "Which die",
            Page::Pot => "How many pots",
            Page::Settings => "Settings",
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
        }
    }
}

impl Settings {
    /// The mode in use: always Dice when the menu has no Mode page.
    pub fn play(&self) -> PlayMode {
        if self.modes {
            self.play
        } else {
            PlayMode::Dice
        }
    }

    /// The die and count a throw rolls in the current mode.
    pub fn active(&self) -> (DieKind, u8) {
        match self.play() {
            PlayMode::Dice => (self.die, self.count),
            PlayMode::PassThePot => (DieKind::PassThePot, self.pot_count),
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
    /// Index into [`SETTINGS`].
    pub setting: u8,
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
            setting: 0,
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
                    Page::Settings => {
                        next.setting = step(self.setting as usize, by, SETTINGS.len()) as u8;
                    }
                }
            }
        }
        next
    }

    /// Holding on this draft restarts the die instead of saving.
    pub fn restart_selected(&self) -> bool {
        self.page == Page::Settings && self.setting == RESTART
    }

    pub fn commit(&self, s: &mut Settings) {
        s.play = self.play;
        s.die = self.die;
        s.count = self.count;
        s.pot_count = self.pot_count;
    }

    /// The setup the draft would roll.
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
            Page::Settings => write!(s, "{}", SETTINGS[self.setting as usize].0),
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
    fn restart_is_the_tenth_setting() {
        let mut d = draft().tipped(TipDir::Left).tipped(TipDir::Left);
        assert_eq!(d.page, Page::Settings);
        for _ in 0..9 {
            assert!(!d.restart_selected());
            d = d.tipped(TipDir::Up);
        }
        assert_eq!(SETTINGS[d.setting as usize].0, "Restart");
        assert!(d.restart_selected());
    }

    #[test]
    fn short_labels() {
        assert_eq!(short_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(short_label(DieKind::PassThePot, 1).as_str(), "Pot ×1");
    }
}
