//! What each display target does differently in the firmware core: how a
//! frame is packed and sent, and which layout draws the screens.
//!
//! [`smokebomb_hal::Target`] says what the panel *is* (size, pixel type,
//! physical size). [`DisplayTarget`] adds what the core does with it. Screen
//! code stays free of target checks: a screen that has a layout of its own
//! on a target goes through a hook here, and the 96×96 layout is the
//! default.
//!
//! On the 64×64 target, screens without a 64×64 design yet (the Nest's
//! clock and guidance, and the save flash, a plain wash) fall back to the
//! 96×96 layout drawn through the 64×64 transform: everything at two thirds
//! of the size, in white. That is a stopgap, not a design; see the README's
//! 30 mm section.

use smokebomb_hal::{AssetStore, Color, Grey96, Mount, Rgb64, Target};
use smokebomb_shared::assets::SpriteKind;
use smokebomb_shared::RollRecord;

use crate::display::Framebuffer;
use crate::menu::{Draft, Setup};
use crate::nest::NestFace;
use crate::panel::Layout;
use crate::pigs::{Locked, Throw, Token};
use crate::screens::{self, Ctx};
use crate::screens64;
use crate::smoke::Special;

/// How a target's frames reach its panels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Delivery {
    /// Every face, whole, every tick: the original behaviour.
    WholeFrames,
    /// Only what changed (8×8 tiles), within a byte budget per tick, the
    /// face down last and no faster than `face_down_ms`.
    Dirty {
        bytes_per_tick: usize,
        face_down_ms: u64,
    },
}

/// A display target as the firmware core uses it.
pub trait DisplayTarget: Target {
    /// How frames are delivered.
    const DELIVERY: Delivery;

    /// The packed frame's layout.
    const LAYOUT: Layout;

    /// One face's tile hashes for dirty tracking ([`crate::panel`]):
    /// `[u32; tiles]`, or `[u32; 0]` for a target that sends whole frames
    /// (so it costs no RAM there).
    type Tiles: Copy + AsRef<[u32]> + AsMut<[u32]> + 'static;
    const NO_TILES: Self::Tiles;

    /// Pack a framebuffer the way the panel takes it, turned for how the
    /// face's panel is mounted (`Target::MOUNT`).
    fn pack(fb: &Framebuffer<Self>, mount: Mount, out: &mut Self::Panel);

    /// The colour of each kind of particle. Grey panels draw them white
    /// (their sprites carry their own grey).
    fn smoke_tint(_kind: SpriteKind, _money: bool) -> Color {
        Color::WHITE
    }

    /// Pig Toss: the pigs' colour (grey panels draw them in their own
    /// shades).
    fn pig_tint() -> Color {
        Color::WHITE
    }

    /// The hold ring, filled `p` (0–1) clockwise from 12 o'clock.
    fn draw_hold_ring<A: AssetStore>(c: &mut Ctx<A, Self>, p: f32, alpha: f32, grow: f32) {
        screens::draw_hold_ring(c, p, alpha, grow);
    }

    /// Hot Potato's lit fuse, `t` seconds on the die's clock (for
    /// animation between ticks; the 96×96 die needs none).
    fn draw_fuse<A: AssetStore>(c: &mut Ctx<A, Self>, heat: f32, pulse: f32, t: f32) {
        let _ = t;
        screens::draw_fuse(c, heat, pulse);
    }

    /// Hot Potato going off, `t` seconds ago.
    fn draw_boom<A: AssetStore>(c: &mut Ctx<A, Self>, t: f32) {
        screens::draw_boom(c, t);
    }

    /// Pig Toss: how the faces frame the pigs.
    fn pig_lens() -> crate::pigfx::Lens {
        crate::pigfx::Lens::WHOLE
    }

    /// Pig Toss: how the settled pigs make way for the score, `s` seconds
    /// after landing: `(shrink, alpha)`. The 96×96 die shrinks them to the
    /// top of the face; a smaller panel can fade them instead.
    fn pig_settle(s: f32) -> (f32, f32) {
        use screens::pig_score::{SHRINK_AT, SHRINK_S};
        let u = ((s - SHRINK_AT) / SHRINK_S).clamp(0.0, 1.0);
        (u * u * (3.0 - 2.0 * u), 1.0)
    }

    // ---------- screens with a layout per target ----------

    fn draw_pigs_label<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        setup: Setup,
        token: Token,
        won: bool,
        alpha: f32,
    ) {
        screens::draw_pigs_label(c, setup, token, won, alpha);
    }

    fn draw_pig_score<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        throw: &Throw,
        t: f32,
        next: u8,
        tokens: &[Token],
        alpha: f32,
    ) {
        screens::draw_pig_score(c, throw, t, next, tokens, alpha);
    }

    fn draw_locked<A: AssetStore>(c: &mut Ctx<A, Self>, l: &Locked, next: Token, t: f32, fade: f32) {
        screens::draw_locked(c, l, next, t, fade);
    }

    fn draw_pig_win<A: AssetStore>(c: &mut Ctx<A, Self>, winner: Token, total: u16, t: f32, fade: f32) {
        screens::draw_pig_win(c, winner, total, t, fade);
    }

    fn draw_boot<A: AssetStore>(c: &mut Ctx<A, Self>, index: usize, top: bool, t: f32) {
        screens::draw_boot(c, index, top, t);
    }

    /// Sugar Rush between levels.
    fn draw_rush_banner<A: AssetStore>(c: &mut Ctx<A, Self>, banner: crate::rush::Banner, t: f32) {
        screens::draw_rush_banner(c, banner, t);
    }

    fn draw_wake_label<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        setup: Setup,
        label: &str,
        alpha: f32,
        face: usize,
    ) {
        let _ = face;
        screens::draw_wake_label(c, setup, label, alpha);
    }

    fn draw_result<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        record: &RollRecord,
        special: Option<Special>,
        alpha: f32,
    ) {
        screens::draw_result(c, record, special, alpha);
    }

    fn draw_success<A: AssetStore>(c: &mut Ctx<A, Self>, label: &str, nudge: &str, t: f32) {
        screens::draw_success(c, label, nudge, t);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_menu<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        m: &Draft,
        battery: f32,
        ox: f32,
        oy: f32,
        alpha: f32,
        scale: f32,
    ) {
        screens::draw_menu(c, m, battery, ox, oy, alpha, scale);
    }

    fn draw_nest<A: AssetStore>(c: &mut Ctx<A, Self>, face: &NestFace) {
        screens::draw_nest(c, face);
    }

    fn draw_bolt<A: AssetStore>(c: &mut Ctx<A, Self>, alpha: f32) {
        screens::draw_bolt(c, alpha);
    }
}

impl DisplayTarget for Grey96 {
    const DELIVERY: Delivery = Delivery::WholeFrames;
    const LAYOUT: Layout = Layout {
        width: 96,
        height: 96,
        bits: 4,
    };
    type Tiles = [u32; 0];
    const NO_TILES: Self::Tiles = [];

    fn pack(fb: &Framebuffer<Self>, mount: Mount, out: &mut Self::Panel) {
        fb.quantize_mounted(mount, out);
    }
}

/// SPI clock the SSD1357 accepts on its 4-wire interface: t_cycle ≥ 100 ns,
/// so at most 10 MHz (SSD1357 datasheet Rev 1.0, Table 9-4, p. 32).
pub const SSD1357_SPI_HZ: u32 = 10_000_000;
/// Panel buses on the 30 mm board. **Estimate**: two SPIM buses, three faces
/// each; the board isn't drawn yet. At 10 MHz one bus moves 1.25 MB/s, so a
/// full 8 KB frame takes ≈6.6 ms and six faces ≈39 ms: whole frames on one
/// bus top out near 25 fps, which is why this target sends dirty tiles.
pub const RGB64_PANEL_BUSES: u32 = 2;
/// Bus time lost to commands, chip-select and DMA set-up per region.
/// **Estimate**: 15 %.
pub const RGB64_BUS_EFFICIENCY: f32 = 0.85;

impl DisplayTarget for Rgb64 {
    const DELIVERY: Delivery = Delivery::Dirty {
        bytes_per_tick: ((SSD1357_SPI_HZ / 8 * RGB64_PANEL_BUSES) as f32 * RGB64_BUS_EFFICIENCY) as usize
            / crate::TICK_HZ as usize,
        face_down_ms: 250,
    };

    const LAYOUT: Layout = Layout {
        width: 64,
        height: 64,
        bits: 16,
    };
    type Tiles = [u32; 64];
    const NO_TILES: Self::Tiles = [0; 64];

    fn pack(fb: &Framebuffer<Self>, mount: Mount, out: &mut Self::Panel) {
        fb.pack565_mounted(mount, out);
    }

    fn smoke_tint(kind: SpriteKind, money: bool) -> Color {
        use crate::palette64 as pal;
        match kind {
            SpriteKind::Smoke if money => pal::GOLD,
            SpriteKind::Smoke => pal::SUGAR,
            SpriteKind::Ember => pal::EMBER,
            SpriteKind::Gold => pal::GOLD,
            SpriteKind::Fizzle => pal::RED,
        }
    }

    fn draw_boot<A: AssetStore>(c: &mut Ctx<A, Self>, index: usize, top: bool, t: f32) {
        screens64::draw_boot(c, index, top, t);
    }

    fn draw_rush_banner<A: AssetStore>(c: &mut Ctx<A, Self>, banner: crate::rush::Banner, t: f32) {
        screens64::draw_rush_banner(c, banner, t);
    }

    fn pig_tint() -> Color {
        crate::palette64::PINK
    }

    /// Shrunk onto 64×64 the 96×96 table's pigs are a few pixels long, so
    /// the 64×64 face takes the middle of the picture 1:1 (×1.5) and zooms
    /// in again, with the pigs' wandering pulled in to stay on the face.
    /// Landed side by side they wouldn't fit that big, so the pair turns
    /// toward the viewer and the view fits it to the face. Design values,
    /// tuned on the contact sheet.
    fn pig_lens() -> crate::pigfx::Lens {
        crate::pigfx::Lens {
            zoom: 1.3,
            spread: 0.5,
            hop: 0.35,
            drop: -3.0,
            turn: 0.9,
            fit: 60.0,
            crop: true,
        }
    }

    /// Shrunk to the top of a 64×64 face the pigs are specks, so they stay
    /// full size and fade as the score comes in.
    fn pig_settle(s: f32) -> (f32, f32) {
        use screens::pig_score::{SHRINK_AT, SHRINK_S};
        (0.0, 1.0 - ((s - SHRINK_AT) / SHRINK_S).clamp(0.0, 1.0))
    }

    fn draw_pigs_label<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        _setup: Setup,
        token: Token,
        won: bool,
        alpha: f32,
    ) {
        screens64::draw_pigs_label(c, token, won, alpha);
    }

    fn draw_pig_score<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        throw: &Throw,
        t: f32,
        next: u8,
        tokens: &[Token],
        alpha: f32,
    ) {
        screens64::draw_pig_score(c, throw, t, next, tokens, alpha);
    }

    fn draw_locked<A: AssetStore>(c: &mut Ctx<A, Self>, l: &Locked, next: Token, t: f32, fade: f32) {
        screens64::draw_locked(c, l, next, t, fade);
    }

    fn draw_pig_win<A: AssetStore>(c: &mut Ctx<A, Self>, winner: Token, total: u16, t: f32, fade: f32) {
        screens64::draw_pig_win(c, winner, total, t, fade);
    }

    fn draw_wake_label<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        setup: Setup,
        label: &str,
        alpha: f32,
        face: usize,
    ) {
        screens64::draw_idle(c, setup, label, alpha, face);
    }

    fn draw_result<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        record: &RollRecord,
        special: Option<Special>,
        alpha: f32,
    ) {
        screens64::draw_result(c, record, special, alpha);
    }

    fn draw_success<A: AssetStore>(c: &mut Ctx<A, Self>, label: &str, nudge: &str, t: f32) {
        screens64::draw_success(c, label, nudge, t);
    }

    fn draw_menu<A: AssetStore>(
        c: &mut Ctx<A, Self>,
        m: &Draft,
        battery: f32,
        ox: f32,
        oy: f32,
        alpha: f32,
        scale: f32,
    ) {
        screens64::draw_menu(c, m, battery, ox, oy, alpha, scale);
    }

    fn draw_nest<A: AssetStore>(c: &mut Ctx<A, Self>, face: &NestFace) {
        if !screens64::draw_nest(c, face) {
            screens::draw_nest(c, face);
        }
    }

    fn draw_bolt<A: AssetStore>(c: &mut Ctx<A, Self>, alpha: f32) {
        screens64::draw_low_battery(c, alpha);
    }

    fn draw_hold_ring<A: AssetStore>(c: &mut Ctx<A, Self>, p: f32, alpha: f32, grow: f32) {
        screens64::draw_hold_ring(c, p, alpha, grow);
    }

    fn draw_fuse<A: AssetStore>(c: &mut Ctx<A, Self>, heat: f32, pulse: f32, t: f32) {
        screens64::draw_fuse(c, heat, pulse, t);
    }

    fn draw_boom<A: AssetStore>(c: &mut Ctx<A, Self>, t: f32) {
        screens64::draw_boom(c, t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_64_budget_is_below_whole_frames() {
        let Delivery::Dirty { bytes_per_tick, .. } = Rgb64::DELIVERY else {
            panic!("Rgb64 sends dirty tiles");
        };
        // ≈35 KB a tick at 60 Hz on two 10 MHz buses: about four whole
        // faces, not six.
        assert!(
            bytes_per_tick > 8192 * 3 && bytes_per_tick < 8192 * 6,
            "{bytes_per_tick}"
        );
    }

    #[test]
    fn layouts_match_the_panels() {
        use smokebomb_hal::Region;
        assert_eq!(
            Rgb64::LAYOUT.region_bytes(Region::full::<Rgb64>()),
            size_of::<<Rgb64 as Target>::Panel>()
        );
        assert_eq!(
            Grey96::LAYOUT.region_bytes(Region::full::<Grey96>()),
            size_of::<<Grey96 as Target>::Panel>()
        );
        assert_eq!(<Rgb64 as DisplayTarget>::NO_TILES.len(), 64);
    }
}
