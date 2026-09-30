//! The firmware as a static library for the Zephyr app
//! (`packages/firmware/zephyr`). The app's `main()` calls `smokebomb_main`
//! (from `smokebomb-firmware`) or, in the benchmark build, `smokebomb_bench`.
//!
//! On the host this crate is empty: it only means something on the board.

#![cfg_attr(target_os = "none", no_std)]

#[cfg(target_os = "none")]
mod board {
    // Linked for its entry point, `smokebomb_main`, which it exports.
    use smokebomb_firmware as _;

    extern "C" {
        fn sb_print(s: *const u8, len: usize);
        fn sb_panic() -> !;
    }

    /// Print through Zephyr's console.
    pub fn print(s: &str) {
        // SAFETY: the shim reads `len` bytes from `s`, which is valid for them.
        unsafe { sb_print(s.as_ptr(), s.len()) }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        use core::fmt::Write;
        let mut s = heapless::String::<160>::new();
        let _ = writeln!(s, "smokebomb panicked: {info}");
        print(&s);
        // SAFETY: the shim halts the system and never returns.
        unsafe { sb_panic() }
    }
}

#[cfg(all(target_os = "none", feature = "bench"))]
mod bench {
    //! The frame benchmark: times a frame's heaviest work on the board and
    //! prints it (see `smokebomb_core::bench`).

    use core::fmt::Write;
    use core::mem::MaybeUninit;
    use core::ptr::addr_of_mut;

    use smokebomb_core::bench::{run, Bench, FRAMES, FRAME_US};
    use smokebomb_core::display::Framebuffer;
    use smokebomb_core::effects::Effects;
    use smokebomb_hal::{AssetStore, FrameBytes, HalError, HalResult, FRAME_BYTES};

    use super::board::print;

    extern "C" {
        fn sb_uptime_us() -> u64;
    }

    /// The benchmark needs no asset pack: it draws without sprites.
    struct NoAssets;

    impl AssetStore for NoAssets {
        fn capacity(&self) -> u32 {
            0
        }
        fn read(&mut self, _: u32, _: &mut [u8]) -> HalResult<()> {
            Err(HalError::NotReady)
        }
    }

    // Far too big for the main stack: statics, as the firmware's are.
    static mut FRAMES_BUF: [Framebuffer; 6] = [Framebuffer::new(); 6];
    static mut PANEL: FrameBytes = [0; FRAME_BYTES];
    static mut SMOKE: MaybeUninit<Effects> = MaybeUninit::uninit();
    static mut PIGS: MaybeUninit<Effects> = MaybeUninit::uninit();

    /// Run the benchmark once and print the results. Called once, from the
    /// Zephyr app's `main()`.
    #[no_mangle]
    pub extern "C" fn smokebomb_bench() {
        // SAFETY: called once, from one thread, so these are the only
        // references to the statics.
        let (frames, panel, smoke, pigs) = unsafe {
            (
                &mut *addr_of_mut!(FRAMES_BUF),
                &mut *addr_of_mut!(PANEL),
                &mut *addr_of_mut!(SMOKE),
                &mut *addr_of_mut!(PIGS),
            )
        };
        let smoke = Effects::init(smoke, &mut NoAssets, None, 1);
        let pigs = Effects::init(pigs, &mut NoAssets, None, 1);
        let mut b = Bench {
            frames,
            smoke: smoke.smoke().expect("starts as smoke"),
            pigs: pigs.use_pigs(),
            panel,
        };
        print("smokebomb frame benchmark\n");
        // SAFETY: the shim only reads Zephyr's clock.
        let timings = run(&mut b, || unsafe { sb_uptime_us() });
        for t in &timings {
            let mut s = heapless::String::<120>::new();
            let us = t.per_frame_us();
            let _ = write!(
                s,
                "{:<24} {:>7} us a frame  {:>5}% of 60 Hz",
                t.name,
                us,
                us * 100 / FRAME_US
            );
            if t.particles > 0 {
                let _ = write!(s, "  ({} particles)", t.particles);
            }
            s.push('\n').ok();
            print(&s);
        }
        let mut s = heapless::String::<80>::new();
        let _ = writeln!(s, "each over {FRAMES} frames; done");
        print(&s);
    }
}
