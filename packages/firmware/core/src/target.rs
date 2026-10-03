//! What each display target does differently in the firmware core: how a
//! frame is packed and sent, and which layout draws the screens.
//!
//! [`smokebomb_hal::Target`] says what the panel *is* (size, pixel type,
//! physical size). [`DisplayTarget`] adds what the core does with it. Screen
//! code stays free of target checks: a screen that has a layout of its own
//! on a target goes through a hook here, and the 96×96 layout is the
//! default.
//!
//! On the 64×64 target, screens without a 64×64 design yet (Pig Toss, Hot
//! Potato, Pass the Pot's bills, the Nest's clock and guidance, the hold
//! ring and save flash) fall back to the 96×96 layout drawn through the
//! 64×64 transform: everything at two thirds of the size, in white. That is
//! a stopgap, not a design; see the README's 30 mm section.

use smokebomb_hal::{AssetStore, Color, Grey96, Region, Rgb64, Target};
use smokebomb_shared::assets::SpriteKind;
use smokebomb_shared::RollRecord;

use crate::display::Framebuffer;
use crate::menu::{Draft, Setup};
use crate::nest::NestFace;
use crate::screens::{self, Ctx};
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

    /// Pack a framebuffer the way the panel takes it.
    fn pack(fb: &Framebuffer<Self>, out: &mut Self::Panel);

    /// The bytes of `region` within a packed frame, as the panel's address
    /// window takes them (row by row). For the size accounting.
    fn region_bytes(region: Region) -> usize;

    /// The colour of each kind of particle. Grey panels draw them white
    /// (their sprites carry their own grey).
    fn smoke_tint(_kind: SpriteKind, _money: bool) -> Color {
        Color::WHITE
    }

    // ---------- screens with a layout per target ----------

    fn draw_boot<A: AssetStore>(c: &mut Ctx<A, Self>, index: usize, top: bool, t: f32) {
        screens::draw_boot(c, index, top, t);
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

    fn pack(fb: &Framebuffer<Self>, out: &mut Self::Panel) {
        fb.quantize(out);
    }

    /// 4 bpp: a row of the region is half its width in bytes (rounded out to
    /// whole bytes).
    fn region_bytes(region: Region) -> usize {
        let (x0, x1) = (region.x0 as usize / 2, (region.x1 as usize).div_ceil(2));
        (x1 - x0) * region.height()
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

    fn pack(fb: &Framebuffer<Self>, out: &mut Self::Panel) {
        fb.pack565(out);
    }

    fn region_bytes(region: Region) -> usize {
        region.width() * region.height() * 2
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
    fn region_bytes() {
        let r = Region {
            x0: 1,
            y0: 0,
            x1: 4,
            y1: 2,
        };
        assert_eq!(Rgb64::region_bytes(r), 12);
        assert_eq!(Grey96::region_bytes(r), 4);
        assert_eq!(Rgb64::region_bytes(Region::full::<Rgb64>()), 8192);
    }
}
