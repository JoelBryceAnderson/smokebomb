//! A frame's heaviest work, timed piece by piece, so it can be measured on
//! the board before the drivers exist: nothing here touches a peripheral.
//! The Zephyr app runs it in place of the firmware when built with
//! `bench.conf`; a host test runs it too.

use smokebomb_hal::FrameBytes;

use crate::display::Framebuffer;
use crate::orientation::Quarter;
use crate::pigfx::{Canvas, Scene};
use crate::pigs::Pose;
use crate::smoke::Smoke;

/// Frames timed for each piece of work.
pub const FRAMES: u32 = 30;

/// A frame's time at 60 Hz, µs.
pub const FRAME_US: u64 = 1_000_000 / 60;

/// What the work runs on. The caller owns it: on the board it's statics,
/// far too big for the stack.
pub struct Bench<'a> {
    pub frames: &'a mut [Framebuffer; 6],
    pub smoke: &'a mut Smoke,
    pub pigs: &'a mut Canvas,
    pub panel: &'a mut FrameBytes,
}

/// How long one piece of work took.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub name: &'static str,
    /// Over [`FRAMES`] frames, µs.
    pub total_us: u64,
    /// Particles in play, for the smoke.
    pub particles: usize,
}

impl Timing {
    pub fn per_frame_us(&self) -> u64 {
        self.total_us / FRAMES as u64
    }
}

const DT: f32 = 1.0 / 60.0;

const FACES: [Quarter; 6] = [
    Quarter::R0,
    Quarter::R90,
    Quarter::R180,
    Quarter::R270,
    Quarter::R0,
    Quarter::R90,
];

/// Time each piece of work over [`FRAMES`] frames, with `now_us` as the
/// clock.
pub fn run(b: &mut Bench, mut now_us: impl FnMut() -> u64) -> [Timing; 5] {
    let mut time =
        |name, particles: &dyn Fn(&Bench) -> usize, b: &mut Bench, work: &mut dyn FnMut(&mut Bench, u32)| {
            let start = now_us();
            for k in 0..FRAMES {
                work(b, k);
            }
            Timing {
                name,
                total_us: now_us().saturating_sub(start),
                particles: particles(b),
            }
        };
    let count = |b: &Bench| b.smoke.len();
    let none = |_: &Bench| 0;

    // A full cloud of sugar in the air: shaken to full charge and thrown.
    b.smoke.set_up([0.0, 1.0, 0.0]);
    b.smoke.shake_start();
    for _ in 0..120 {
        b.smoke.step(DT);
    }
    b.smoke.throw();
    let smoke_frame = &mut |b: &mut Bench, _| {
        for fb in b.frames.iter_mut() {
            fb.clear();
        }
        b.smoke.step(DT);
        b.smoke.draw(b.frames);
    };
    let in_air = time("sugar in the air", &count, b, smoke_frame);
    // Landed: the top-up and the embers join it.
    b.smoke.land();
    let landing = time("sugar landing", &count, b, smoke_frame);

    // The pigs in the air, cast every frame, laid onto all six faces.
    let tumbling = time("pigs tumbling", &none, b, &mut |b, k| {
        for fb in b.frames.iter_mut() {
            fb.clear();
        }
        b.pigs.draw(Scene::Tumbling(k as f32 * DT));
        for (fb, rot) in b.frames.iter_mut().zip(FACES) {
            b.pigs.lay_onto(fb, rot, 1.0);
        }
    });
    // At rest under the score: cast once, then only laid on.
    let at_rest = time("pigs at rest", &none, b, &mut |b, _| {
        for fb in b.frames.iter_mut() {
            fb.clear();
        }
        b.pigs.draw(Scene::Settling {
            land: 1.0,
            u: 1.0,
            shrink: 1.0,
            poses: [Pose::Feet, Pose::Back],
            touching: false,
        });
        for (fb, rot) in b.frames.iter_mut().zip(FACES) {
            b.pigs.lay_onto(fb, rot, 1.0);
        }
    });

    // Every frame, whatever is on the faces: dither to the panel's 16
    // levels and pack each face for the display.
    let dither = time("dither and pack 6 faces", &none, b, &mut |b, _| {
        for fb in b.frames.iter() {
            fb.quantize(b.panel);
        }
    });
    [in_air, landing, tumbling, at_rest, dither]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::Effects;
    use core::mem::MaybeUninit;

    #[test]
    fn every_piece_does_its_work() {
        use smokebomb_hal::{AssetStore, HalError, HalResult, FRAME_BYTES};
        struct NoAssets;
        impl AssetStore for NoAssets {
            fn capacity(&self) -> u32 {
                0
            }
            fn read(&mut self, _: u32, _: &mut [u8]) -> HalResult<()> {
                Err(HalError::NotReady)
            }
        }
        let mut smoke_fx = MaybeUninit::uninit();
        let smoke_fx = Effects::init(&mut smoke_fx, &mut NoAssets, None, 1);
        let mut pigs_fx = MaybeUninit::uninit();
        let pigs_fx = Effects::init(&mut pigs_fx, &mut NoAssets, None, 1);
        let mut frames = [Framebuffer::new(); 6];
        let mut panel = [0u8; FRAME_BYTES];
        let mut b = Bench {
            frames: &mut frames,
            smoke: smoke_fx.smoke().unwrap(),
            pigs: pigs_fx.use_pigs(),
            panel: &mut panel,
        };
        let start = std::time::Instant::now();
        let t = run(&mut b, || start.elapsed().as_micros() as u64);
        for r in &t {
            std::println!(
                "{}: {} µs a frame, {} particles",
                r.name,
                r.per_frame_us(),
                r.particles
            );
        }
        assert!(
            t[0].particles >= 380,
            "a full cloud in the air: {}",
            t[0].particles
        );
        assert!(t[1].particles > t[0].particles, "the landing adds to it");
        assert!(b.panel.iter().any(|&p| p != 0), "the pigs reached the panel");
    }
}
