//! On-die setup menu (SIM_SPEC C3).
//!
//! Hold any screen to open the menu there. Tipping the die moves through it:
//! left and right turn the page, up and down change the value. Everything
//! happens on a [`Draft`]; holding again saves it, and a throw, docking or
//! 25 s without input leaves the setup as it was.

use core::fmt::Write as _;

use heapless::String;
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
}

impl Default for Settings {
    fn default() -> Self {
        // The mockup's default setup: a single d20.
        Self {
            die: DieKind::D20,
            count: 1,
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
}

impl Draft {
    /// The menu always opens on its first page.
    pub fn new(s: &Settings) -> Self {
        Self {
            page: Page::Count,
            die: s.die,
            count: s.count,
            setting: 0,
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

    /// Holding on this draft restarts the die instead of saving.
    pub fn restart_selected(&self) -> bool {
        self.page == Page::Settings && self.setting == RESTART
    }

    pub fn commit(&self, s: &mut Settings) {
        s.die = self.die;
        s.count = self.count;
    }

    /// The page's big value (not used on the Settings page).
    pub fn value(&self) -> String<16> {
        let mut s = String::new();
        let _ = match self.page {
            Page::Count => write!(s, "{}", self.count),
            Page::Die => write!(s, "{}", die_name(self.die)),
            Page::Settings => write!(s, "{}", SETTINGS[self.setting as usize].0),
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
        assert_eq!(SETTINGS[d.setting as usize].0, "Restart");
        assert!(d.restart_selected());
    }

    #[test]
    fn short_labels() {
        assert_eq!(short_label(DieKind::D6, 3).as_str(), "3d6");
        assert_eq!(short_label(DieKind::PassThePot, 1).as_str(), "Pot ×1");
    }
}
