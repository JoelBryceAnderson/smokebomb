//! Firmware image entry point.
//!
//! Selects the board at compile time:
//!
//! ```text
//! cargo build -p smokebomb-firmware --features simulator
//! cargo build -p smokebomb-firmware --no-default-features --features nrf54l15 \
//!     --target thumbv8m.main-none-eabihf      # or: cargo fw
//! ```
//!
//! With neither feature the crate only re-exports the core, which is what
//! `cargo check --workspace` sees.

#![cfg_attr(not(feature = "simulator"), no_std)]

#[cfg(all(feature = "simulator", feature = "nrf54l15"))]
compile_error!("features `simulator` and `nrf54l15` are mutually exclusive");

pub use smokebomb_core;
pub use smokebomb_core::Firmware;

#[cfg(feature = "simulator")]
pub mod board {
    //! Simulator board. The caller keeps the [`SimHandle`] to drive inputs.
    pub use smokebomb_hal_simulator::{SimHandle, SimPlatform as Board};

    pub type Firmware = smokebomb_core::Firmware<Board>;

    pub fn boot(handle: &SimHandle) -> smokebomb_hal::HalResult<Firmware> {
        Firmware::new(handle.peripherals())
    }
}

#[cfg(feature = "nrf54l15")]
pub mod board {
    //! Production board. Zephyr's `main()` calls [`smokebomb_main`].
    pub use smokebomb_hal_nrf54l15::Nrf54l15 as Board;

    pub type Firmware = smokebomb_core::Firmware<Board>;

    use core::mem::MaybeUninit;
    use core::ptr::addr_of_mut;
    use core::sync::atomic::{AtomicBool, Ordering};

    /// Where the firmware lives. It is ~115 KB and the main thread's stack
    /// is 8 KB (`CONFIG_MAIN_STACK_SIZE`), so it can't be a local: it sits
    /// in `.bss` and [`Firmware::init`] builds it in place.
    static mut FIRMWARE: MaybeUninit<Firmware> = MaybeUninit::uninit();

    /// The most RAM the firmware's state may take. The app core has 188 KB
    /// (the rest of the chip's 256 KB is the FLPR core's), and Zephyr,
    /// Bluetooth and NFC took 50,644 B of it in a build of the Zephyr app
    /// for the nRF54L15 DK, leaving about 141,900 B. This leaves ~20 KB of
    /// that for the Rust code's own statics, the stacks and some margin.
    ///
    /// If a change trips this, don't just raise it: a game's per-frame
    /// visuals belong in `smokebomb_core::effects::Effects`, which holds one
    /// mode's at a time, rather than in a field of their own. Raise it only
    /// with a measured build that shows the room is there.
    pub const RAM_BUDGET: usize = 120 * 1024;

    const _: () = assert!(
        core::mem::size_of::<Firmware>() <= RAM_BUDGET,
        "Firmware is over its RAM budget: see RAM_BUDGET in packages/firmware/src/lib.rs"
    );
    static BOOTED: AtomicBool = AtomicBool::new(false);

    /// Boots the firmware into its static slot. Only the first call gets it;
    /// later calls fail with [`HalError::NotReady`](smokebomb_hal::HalError).
    pub fn boot() -> smokebomb_hal::HalResult<&'static mut Firmware> {
        if BOOTED.swap(true, Ordering::AcqRel) {
            return Err(smokebomb_hal::HalError::NotReady);
        }
        // SAFETY: `BOOTED` lets only one caller past, once, so this is the
        // only reference to `FIRMWARE` there will ever be.
        let slot = unsafe { &mut *addr_of_mut!(FIRMWARE) };
        Firmware::init(slot, smokebomb_hal_nrf54l15::peripherals())
    }

    /// Entry point exported to the Zephyr C application. Returns only if boot
    /// fails, with a negative error code.
    #[no_mangle]
    pub extern "C" fn smokebomb_main() -> i32 {
        let Ok(fw) = boot() else {
            return -1;
        };
        loop {
            let _ = fw.tick();
            // TODO: k_msleep(1000 / TICK_HZ) via the Zephyr shim.
        }
    }
}
