//! On-die settings menu (hold a face to enter, tip to change page, tap to
//! change the value, hold again to save).

use smokebomb_shared::DieKind;

use crate::display::{Framebuffer, FG};
use crate::orientation::Quarter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPage {
    DieCount,
    DieType,
    Brightness,
    About,
}

impl MenuPage {
    pub const fn first() -> Self {
        MenuPage::DieCount
    }

    pub const fn next(self) -> Self {
        match self {
            MenuPage::DieCount => MenuPage::DieType,
            MenuPage::DieType => MenuPage::Brightness,
            MenuPage::Brightness => MenuPage::About,
            MenuPage::About => MenuPage::DieCount,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuInput {
    /// Advance the value shown on this page.
    Cycle(MenuPage),
}

/// User settings. Persisted to internal flash on hardware (TODO).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub die: DieKind,
    pub count: u8,
    pub brightness: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            die: DieKind::D6,
            count: 1,
            brightness: 200,
        }
    }
}

impl Settings {
    pub fn apply(&mut self, input: MenuInput) {
        match input {
            MenuInput::Cycle(MenuPage::DieCount) => {
                self.count = if self.count as usize >= self.die.max_count() {
                    1
                } else {
                    self.count + 1
                };
            }
            MenuInput::Cycle(MenuPage::DieType) => {
                let i = DieKind::ALL.iter().position(|d| *d == self.die).unwrap_or(0);
                self.die = DieKind::ALL[(i + 1) % DieKind::ALL.len()];
                // Choosing Pass the Pot clamps the count to 3 (SIM_SPEC C3).
                self.count = self.count.min(self.die.max_count() as u8);
            }
            MenuInput::Cycle(MenuPage::Brightness) => {
                self.brightness = self.brightness.wrapping_add(64).max(32);
            }
            MenuInput::Cycle(MenuPage::About) => {}
        }
    }
}

/// Placeholder rendering: a page indicator bar across the top plus the value.
pub fn render(fb: &mut Framebuffer, page: MenuPage, s: &Settings, rot: Quarter) {
    let idx = page as usize;
    for i in 0..4 {
        fb.fill_rect(12 + i * 20, 6, 12, 3, if i == idx { FG } else { 48 }, rot);
    }
    let value = match page {
        MenuPage::DieCount => s.count as u16,
        MenuPage::DieType => s.die.sides() as u16,
        MenuPage::Brightness => s.brightness as u16,
        MenuPage::About => 1, // firmware major version
    };
    fb.draw_number(value, 5, FG, rot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use smokebomb_shared::types::MAX_DICE;

    #[test]
    fn die_count_wraps() {
        let mut s = Settings {
            count: MAX_DICE as u8,
            ..Settings::default()
        };
        s.apply(MenuInput::Cycle(MenuPage::DieCount));
        assert_eq!(s.count, 1);
    }

    #[test]
    fn choosing_pass_the_pot_clamps_the_count() {
        let mut s = Settings {
            die: DieKind::D100,
            count: 7,
            ..Settings::default()
        };
        s.apply(MenuInput::Cycle(MenuPage::DieType));
        assert_eq!(s.die, DieKind::PassThePot);
        assert_eq!(s.count, 3);
    }
}
