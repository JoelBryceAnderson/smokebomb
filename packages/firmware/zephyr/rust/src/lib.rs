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

#[cfg(all(target_os = "none", feature = "bringup"))]
mod bringup {
    //! The bench bring-up: says what answers on the I2C bus and which
    //! drivers came up, then cycles test patterns across the panels one at
    //! a time while streaming the IMU and touch readings. When something
    //! doesn't work on the bench, this says whether it's the wiring or the
    //! code. The panels, IMU and touch go through the production HAL.

    use core::fmt::Write;

    use smokebomb_hal::{
        Display, Face, FrameBytes, Imu, Touch, FACE_COUNT, FRAME_BYTES, PANEL_HEIGHT, PANEL_WIDTH,
    };
    use smokebomb_hal_nrf54l15::{CapTouch, Lsm6dsx, Ssd1317Array};

    use super::board::print;

    extern "C" {
        fn sb_uptime_us() -> u64;
        fn sb_sleep_us(us: u32);
        fn sb_i2c_probe(addr: u8) -> i32;
        fn sb_i2c_reg_read(addr: u8, reg: u8, val: *mut u8) -> i32;
        fn sb_device_ready(which: u8) -> i32;
        fn sb_haptics_play(effect: u8) -> i32;
    }

    /// A part the bench may have on its I2C bus, and the register that
    /// says it's that part: `(register, mask, expected)`.
    struct Part {
        addr: u8,
        name: &'static str,
        id: Option<(u8, u8, u8)>,
    }

    const PARTS: &[Part] = &[
        Part {
            addr: 0x6A,
            name: "LSM6DSOX IMU",
            id: Some((0x0F, 0xFF, 0x6C)),
        },
        Part {
            addr: 0x6B,
            name: "LSM6DSOX IMU (address jumper)",
            id: Some((0x0F, 0xFF, 0x6C)),
        },
        Part {
            addr: 0x29,
            name: "CAP1188 touch",
            id: Some((0xFD, 0xFF, 0x50)),
        },
        Part {
            addr: 0x5A,
            name: "DRV2605L haptics",
            id: Some((0x00, 0xE0, 0xE0)),
        },
        Part {
            addr: 0x30,
            name: "MMC5603 magnetometer",
            id: Some((0x39, 0xFF, 0x10)),
        },
        Part {
            addr: 0x1E,
            name: "LIS2MDL magnetometer",
            id: Some((0x4F, 0xFF, 0x40)),
        },
        Part {
            addr: 0x60,
            name: "ATECC608 secure element",
            id: None,
        },
    ];

    /// The devicetree's devices, in `sb_device_ready`'s order.
    const DEVICES: &[&str] = &["imu", "touch", "haptics", "mag_mmc5603", "mag_lis2mdl"];

    /// How long each pattern stays on a face, and between readings.
    const PATTERN_MS: u64 = 1000;
    const READING_MS: u32 = 100;

    fn line(args: core::fmt::Arguments) {
        let mut s = heapless::String::<160>::new();
        let _ = s.write_fmt(args);
        let _ = s.push('\n');
        print(&s);
    }

    fn scan() {
        line(format_args!("i2c: scanning the STEMMA bus"));
        let mut found = 0;
        for addr in 0x08..=0x77u8 {
            // SAFETY: the shim reads one byte from the bus into its own buffer.
            if unsafe { sb_i2c_probe(addr) } != 0 {
                continue;
            }
            found += 1;
            let Some(part) = PARTS.iter().find(|p| p.addr == addr) else {
                line(format_args!(
                    "  0x{addr:02x}  answers (not a part the bench expects)"
                ));
                continue;
            };
            let Some((reg, mask, want)) = part.id else {
                line(format_args!("  0x{addr:02x}  {}", part.name));
                continue;
            };
            let mut got = 0;
            // SAFETY: the shim writes one byte to `got`.
            let err = unsafe { sb_i2c_reg_read(addr, reg, &mut got) };
            let verdict = match err {
                0 if got & mask == want => "id ok",
                0 => "WRONG ID",
                _ => "id read failed",
            };
            line(format_args!(
                "  0x{addr:02x}  {}: reg 0x{reg:02x} = 0x{got:02x}, {verdict}",
                part.name
            ));
        }
        // SAFETY (each probe): as above.
        let answers = |addr| unsafe { sb_i2c_probe(addr) } == 0;
        for (addr, name) in [
            (0x6A, "LSM6DSOX IMU"),
            (0x29, "CAP1188 touch"),
            (0x5A, "DRV2605L haptics"),
        ] {
            if !answers(addr) {
                line(format_args!("  0x{addr:02x}  {name}: MISSING"));
            }
        }
        if !answers(0x30) && !answers(0x1E) {
            line(format_args!(
                "  magnetometer: MISSING (neither MMC5603 at 0x30 nor LIS2MDL at 0x1e)"
            ));
        }
        line(format_args!("i2c: {found} answered"));
    }

    fn devices() {
        for (i, name) in DEVICES.iter().enumerate() {
            // SAFETY: the shim only asks Zephyr whether a device is ready.
            let state = match unsafe { sb_device_ready(i as u8) } {
                1 => "ready",
                0 => "FAILED its init",
                _ => "not in the devicetree",
            };
            line(format_args!("driver {name}: {state}"));
        }
    }

    /// Set one pixel of a 4bpp frame to full.
    fn lit(frame: &mut FrameBytes, x: usize, y: usize) {
        let i = y * PANEL_WIDTH + x;
        frame[i / 2] |= if i % 2 == 0 { 0xF0 } else { 0x0F };
    }

    /// The test patterns, in the order each face shows them: which face it
    /// is and which way up (a block in its top-left corner, a border, and
    /// one bar per face index plus one), every pixel lit, and a checkerboard
    /// of 8-pixel squares.
    const PATTERNS: usize = 3;

    fn pattern(face: usize, n: usize, frame: &mut FrameBytes) {
        *frame = [0; FRAME_BYTES];
        for y in 0..PANEL_HEIGHT {
            for x in 0..PANEL_WIDTH {
                let on = match n {
                    0 => {
                        let border = x == 0 || y == 0 || x == PANEL_WIDTH - 1 || y == PANEL_HEIGHT - 1;
                        let corner = x < 16 && y < 16;
                        let bar =
                            (32..80).contains(&y) && x >= 12 && (x - 12) % 12 < 6 && (x - 12) / 12 <= face;
                        border || corner || bar
                    }
                    1 => true,
                    _ => (x / 8 + y / 8) % 2 == 0,
                };
                if on {
                    lit(frame, x, y);
                }
            }
        }
    }

    const FACES: [Face; FACE_COUNT] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    static mut FRAME: FrameBytes = [0; FRAME_BYTES];

    /// Run the bring-up; never returns. Called once, from the Zephyr app's
    /// `main()`.
    #[no_mangle]
    pub extern "C" fn smokebomb_bringup() {
        // SAFETY: called once, from one thread: the only reference.
        let frame = unsafe { &mut *core::ptr::addr_of_mut!(FRAME) };
        line(format_args!("smokebomb bench bring-up"));
        scan();
        devices();
        // Effect 1 of the LRA library: a strong click.
        // SAFETY: the shim drives the DRV2605L through Zephyr.
        let err = unsafe { sb_haptics_play(1) };
        line(format_args!(
            "haptics: click {}",
            if err == 0 { "sent" } else { "FAILED" }
        ));

        let mut display = Ssd1317Array::new();
        let panels = display.set_enabled(true);
        line(format_args!("panels: reset and set up: {panels:?}"));
        let (mut imu, mut touch) = (Lsm6dsx, CapTouch);

        let mut shown = None;
        // SAFETY: the shim only reads Zephyr's clock.
        let start = unsafe { sb_uptime_us() } / 1000;
        loop {
            // SAFETY: as above.
            let step = ((unsafe { sb_uptime_us() } / 1000 - start) / PATTERN_MS) as usize;
            let (face, n) = (step / PATTERNS % FACE_COUNT, step % PATTERNS);
            if shown != Some((face, n)) {
                if let Some((prev, _)) = shown {
                    if prev != face {
                        *frame = [0; FRAME_BYTES];
                        let _ = display.write_frame(FACES[prev], frame);
                    }
                }
                pattern(face, n, frame);
                let r = display.write_frame(FACES[face], frame);
                line(format_args!("panel {:?}: pattern {n}: {r:?}", FACES[face]));
                shown = Some((face, n));
            }
            let imu = imu.read();
            let touch = touch.read();
            match (imu, touch) {
                (Ok(Some(s)), Ok(t)) => line(format_args!(
                    "imu accel {:6} {:6} {:6} mg  gyro {:8} {:8} {:8} mdps  touch {t:06b}",
                    s.accel_mg[0],
                    s.accel_mg[1],
                    s.accel_mg[2],
                    s.gyro_mdps[0],
                    s.gyro_mdps[1],
                    s.gyro_mdps[2],
                )),
                (imu, touch) => line(format_args!("imu {imu:?}  touch {touch:?}")),
            }
            // SAFETY: sleeps this thread.
            unsafe { sb_sleep_us(READING_MS * 1000) };
        }
    }
}
