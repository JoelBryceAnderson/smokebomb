//! On-die setup menu (SIM_SPEC C3).
//!
//! Hold any screen to open the menu there. Tipping the die moves through it:
//! left and right turn the page, up and down change the value (on the
//! Settings page, up and down pick the item). A tap changes the selected
//! Settings item, or restarts the die on Restart. Everything happens on a
//! [`Draft`]; a hold always saves it and returns to the roll, and a throw,
//! docking or 25 s without input leaves the setup as it was.

use core::fmt::Write as _;

use heapless::String;
use smokebomb_shared::DieKind;

use crate::tips::TipDir;

/// One row of the Settings page. A tap steps `options`; a row without any is
/// display-only (or, for [`RESTART`], an action).
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
    Item::fixed("Restart", &["Tap to restart"]),
    Item::fixed("About", &["v0.1.0 · SB-0042"]),
];

const BRIGHTNESS: usize = 0;
const HAPTICS: usize = 1;
const BLUETOOTH: usize = 6;
const RESTART: u8 = 9;

/// The chosen option of every Settings item.
pub type Choices = [u8; SETTINGS.len()];

fn default_choices() -> Choices {
    let mut c = [0; SETTINGS.len()];
    for (c, item) in c.iter_mut().zip(&SETTINGS) {
        *c = item.default;
    }
    c
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Count,
    Die,
    Settings,
}

impl Page {
    pub const ALL: [Page; 3] = [Page::Count, Page::Die, Page::Settings];

    pub const fn title(self) -> &'static str {
        match self {
            Page::Count => "How many dice",
            Page::Die => "Which die",
            Page::Settings => "Settings",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    fn step(self, by: isize) -> Page {
        Page::ALL[step(self.index(), by, Page::ALL.len())]
    }
}

/// User settings. Persisted to internal flash on hardware (TODO).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub die: DieKind,
    pub count: u8,
    /// The chosen option of each [`SETTINGS`] item.
    pub choices: Choices,
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
}

impl Default for Settings {
    fn default() -> Self {
        // The mockup's default setup: a single d20.
        Self {
            die: DieKind::D20,
            count: 1,
            choices: default_choices(),
        }
    }
}

/// The menu's working copy of the setup, plus where the menu is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Draft {
    pub page: Page,
    pub die: DieKind,
    pub count: u8,
    /// Index into [`SETTINGS`].
    pub setting: u8,
    pub choices: Choices,
}

impl Draft {
    /// The menu always opens on its first page.
    pub fn new(s: &Settings) -> Self {
        Self {
            page: Page::Count,
            die: s.die,
            count: s.count,
            setting: 0,
            choices: s.choices,
        }
    }

    /// The draft after a tip: left is the next page, right the previous; up
    /// is the next value, down the previous. Values wrap around.
    pub fn tipped(self, dir: TipDir) -> Self {
        let mut next = self;
        match dir {
            TipDir::Left => next.page = self.page.step(1),
            TipDir::Right => next.page = self.page.step(-1),
            TipDir::Up | TipDir::Down => {
                let by = if dir == TipDir::Up { 1 } else { -1 };
                match self.page {
                    Page::Count => {
                        let n = self.die.max_count();
                        next.count = step(self.count as usize - 1, by, n) as u8 + 1;
                    }
                    Page::Die => {
                        let i = DieKind::ALL.iter().position(|d| *d == self.die).unwrap_or(0);
                        next.die = DieKind::ALL[step(i, by, DieKind::ALL.len())];
                        // Pass the Pot uses at most three dice.
                        next.count = next.count.min(next.die.max_count() as u8);
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

    /// A tap on this draft restarts the die.
    pub fn restart_selected(&self) -> bool {
        self.page == Page::Settings && self.setting == RESTART
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

    pub fn commit(&self, s: &mut Settings) {
        s.die = self.die;
        s.count = self.count;
        s.choices = self.choices;
    }

    /// The selected Settings item's name and current value.
    pub fn setting(&self) -> (&'static str, &'static str) {
        let item = &SETTINGS[self.setting as usize];
        let i = self.choices[self.setting as usize] as usize;
        (item.name, item.options[i.min(item.options.len() - 1)])
    }

    /// The page's big value (not used on the Settings page).
    pub fn value(&self) -> String<16> {
        let mut s = String::new();
        let _ = match self.page {
            Page::Count => write!(s, "{}", self.count),
            Page::Die => write!(s, "{}", die_name(self.die)),
            Page::Settings => write!(s, "{}", self.setting().0),
        };
        s
    }
}

fn step(i: usize, by: isize, n: usize) -> usize {
    (i as isize + by).rem_euclid(n as isize) as usize
}

/// `d20`, or `Pass the Pot`.
pub fn die_name(die: DieKind) -> &'static str {
    match die {
        DieKind::PassThePot => "Pass the Pot",
        d => d.wire_name(),
    }
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

    #[test]
    fn left_and_right_turn_the_page() {
        let d = draft();
        assert_eq!(d.tipped(TipDir::Left).page, Page::Die);
        assert_eq!(d.tipped(TipDir::Right).page, Page::Settings);
        assert_eq!(
            d.tipped(TipDir::Left).tipped(TipDir::Left).tipped(TipDir::Left),
            d
        );
    }

    #[test]
    fn up_and_down_change_the_value_and_wrap() {
        let d = draft();
        assert_eq!(d.tipped(TipDir::Up).count, 2);
        assert_eq!(d.tipped(TipDir::Down).count, 10);
        let die = d.tipped(TipDir::Left);
        assert_eq!(die.tipped(TipDir::Up).die, DieKind::D100);
        assert_eq!(die.tipped(TipDir::Down).die, DieKind::D12);
    }

    #[test]
    fn choosing_pass_the_pot_clamps_the_count() {
        let d = Draft {
            page: Page::Die,
            die: DieKind::D100,
            count: 7,
            setting: 0,
            choices: default_choices(),
        };
        let pot = d.tipped(TipDir::Up);
        assert_eq!(pot.die, DieKind::PassThePot);
        assert_eq!(pot.count, 3);
        let count = Draft {
            page: Page::Count,
            ..pot
        };
        assert_eq!(count.tipped(TipDir::Up).count, 1, "1–3 for Pass the Pot");
    }

    #[test]
    fn restart_is_the_tenth_setting() {
        let mut d = draft().tipped(TipDir::Right);
        for _ in 0..9 {
            assert!(!d.restart_selected());
            d = d.tipped(TipDir::Up);
        }
        assert_eq!(d.setting().0, "Restart");
        assert!(d.restart_selected());
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
    fn a_tap_steps_the_selected_setting_and_wraps() {
        let mut d = draft().tipped(TipDir::Right); // Settings, on Brightness
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
        let mut d = draft().tipped(TipDir::Right);
        for name in ["Owner", "Restart", "About"] {
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
        let d = draft().tipped(TipDir::Right).tapped().tipped(TipDir::Up).tapped();
        d.commit(&mut s);
        assert_eq!(s.brightness_pct(), 100);
        assert!(!s.haptics_on());
    }

    #[test]
    fn short_labels() {
        assert_eq!(short_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(short_label(DieKind::PassThePot, 1).as_str(), "Pot ×1");
    }
}
