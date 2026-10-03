//! The 64×64 colour panel's palette: every colour its screens use, here and
//! nowhere else.
//!
//! The 96×96 grey die says things with words and space ("max 2d20", a
//! greyed-out "dud"). At 64×64 there is less room, so colour carries the
//! meaning instead, and each colour means one thing:
//!
//! | Colour | Means |
//! |---|---|
//! | [`WHITE`] | the thing to read: a result, a value |
//! | [`DIM`] | secondary: the setup under a result, titles, units |
//! | [`GOLD`] | the best roll: max / crit, and the gold ring of sparks |
//! | [`RED`] | the worst roll: fumble / min, and a battery that needs charging |
//! | [`VIOLET`] | the die itself: its icon, the menu's arrows, the sugar |
//! | [`MINT`] | good news that isn't a roll: charging, saved, a + modifier |
//! | [`EMBER`] | warmth and warning: embers, a − modifier, the low-battery bolt |
//!
//! Values are sRGB as the panel is driven (RGB565 keeps 5/6/5 bits of them).
//! The emitters' real colour points and the module's white balance aren't
//! known yet (NHD-0.6-6464G datasheet), so these are design values to be
//! re-tuned on the panel.

use smokebomb_hal::Color;

pub const BLACK: Color = Color::BLACK;
/// The mockup's foreground white `#F4F5F7`.
pub const WHITE: Color = Color::hex(0xF4F5F7);
/// Secondary text and lines: the mockup's dud grey `#8A8C90`.
pub const DIM: Color = Color::hex(0x8A8C90);
/// Dimmer still, for inactive dots and rules.
pub const FAINT: Color = Color::hex(0x3A3C40);
/// Max / crit: the mockup's colour-panel gold `#F5C451`.
pub const GOLD: Color = Color::hex(0xF5C451);
/// Fumble / min.
pub const RED: Color = Color::hex(0xF0524A);
/// The die and its sugar: the mockup's v2 smoke violet `#B0A8FF`.
pub const VIOLET: Color = Color::hex(0xB0A8FF);
/// Charging, saved, a plus.
pub const MINT: Color = Color::hex(0x5FE0B0);
/// Embers (the mockup's v2 ember `#FF9646`), a minus, the bolt.
pub const EMBER: Color = Color::hex(0xFF9646);

/// The sugar crystals: a pale violet-white, so the cloud reads as sugar and
/// not as text.
pub const SUGAR: Color = Color::hex(0xD8D4FF);
