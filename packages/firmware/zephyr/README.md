# Zephyr application shell (not built yet)

The production image is a Zephyr application whose `main()` hands control to
the Rust firmware (`smokebomb_main` in `packages/firmware/src/lib.rs`). Zephyr
owns drivers, the BLE stack, MCUboot/DFU and the scheduler; Rust owns all
application logic through the HAL traits.

Planned build (once the Rust static library is wired in):

```sh
west init -l packages/firmware/zephyr && west update
cargo fw                                   # builds the Rust side for thumbv8m.main-none-eabihf
west build -b nrf54l15dk/nrf54l15/cpuapp packages/firmware/zephyr
```

Nothing in CI builds this directory yet.
