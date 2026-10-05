//! Text sizes (SMOKEBOMB_SIM_UPDATE_BRIEF_3, part 1).
//!
//! Cap height ≥ viewing distance ÷ 200. Table screens (read by everyone
//! after a roll, 0.5–1.2 m) have two tiers: T1, the answer in 1–3
//! characters, and T2, one word of detail in at most 5. Held screens (read
//! by whoever holds the die, 0.25–0.35 m) have H1, the value being chosen,
//! and H2, titles and labels in at most 10 characters. Nothing smaller is
//! allowed, a table screen has at most one T1 and one T2, and no line wraps.
//!
//! Sizes are cap heights in panel px, for each panel's pitch. The painter
//! notes every text it draws ([`crate::gfx::Mark`]), and the firmware keeps
//! each face's notes with what kind of screen it was ([`FaceAudit`]), so a
//! test can check every screen against these.

use crate::gfx::Marks;

/// One panel's tiers: cap heights in panel px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tiers {
    /// T1's target, for 1–2 characters.
    pub t1: f32,
    /// T1's floor, which 3 characters may drop to.
    pub t1_floor: f32,
    pub t2: f32,
    pub h1: f32,
    pub h2: f32,
    /// The widest a line may be, panel px: the lit area less a margin.
    pub line: f32,
}

/// The 30 mm die: 64×64 at 0.168 mm.
pub const RGB64: Tiers = Tiers {
    t1: 30.0,
    t1_floor: 21.0,
    t2: 15.0,
    h1: 18.0,
    h2: 7.0,
    line: 58.0,
};

/// The 34 mm die: 96×96 at 0.18 mm.
pub const GREY96: Tiers = Tiers {
    t1: 28.0,
    t1_floor: 20.0,
    t2: 14.0,
    h1: 17.0,
    h2: 7.0,
    line: 87.0,
};

/// Most characters in each kind of text.
pub const T1_CHARS: u8 = 3;
pub const T2_CHARS: u8 = 5;
pub const H2_CHARS: u8 = 10;

/// What kind of screen a face showed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    /// Read by the table: results, the setup label, game state.
    Table,
    /// Read by whoever holds the die: the menu, the success screen, a
    /// hold's preview.
    Held,
    /// Not text to read: boot, the Nest, blank.
    #[default]
    Other,
}

/// One face's last frame, for the size audit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaceAudit {
    pub screen: Screen,
    pub marks: Marks,
}
