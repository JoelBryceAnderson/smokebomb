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

    pub fn boot() -> smokebomb_hal::HalResult<Firmware> {
        Firmware::new(smokebomb_hal_nrf54l15::peripherals())
    }

    /// Entry point exported to the Zephyr C application. Returns only if boot
    /// fails, with a negative error code.
    #[no_mangle]
    pub extern "C" fn smokebomb_main() -> i32 {
        let Ok(mut fw) = boot() else {
            return -1;
        };
        loop {
            let _ = fw.tick();
            // TODO: k_msleep(1000 / TICK_HZ) via the Zephyr shim.
        }
    }
}
