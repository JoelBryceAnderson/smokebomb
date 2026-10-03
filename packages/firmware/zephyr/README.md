# Zephyr application

The production image is a Zephyr application whose `main()` hands control to
the Rust firmware (`smokebomb_main` in `packages/firmware/src/lib.rs`). Zephyr
owns drivers, the BLE stack, MCUboot/DFU and the scheduler; Rust owns all
application logic through the HAL traits.

- `rust/` builds the firmware as a static library (`smokebomb-zephyr`): the
  panic handler, and in the benchmark build, `smokebomb_bench`.
  `CMakeLists.txt` runs cargo for it and links it into the app.
- `src/shim.c` wraps the Zephyr calls Rust makes (the clock, sleeping,
  printing), since most of Zephyr's API is inline functions and macros.
  `src/board.c` does the same for the board's devices: the panels' SPI
  bus, the IMU, touch, haptics and the I2C bus.
- `boards/nrf54l15dk_nrf54l15_cpuapp.overlay` is the bench's devicetree:
  the panels, the STEMMA QT chain and the pins they're on. `BENCH.md` is
  the same pin map as a wiring table, with the SPI budget.
- `sysbuild.conf` turns on MCUboot. Where updates are staged (the second slot
  in internal flash, or the external QSPI flash) is still to decide: it sets
  the app's flash. This is NCS's default, internal, which gives the app a
  682 KB slot.
- `prj.conf` turns on the FPU with the hard-float ABI, to match Rust's
  `thumbv8m.main-none-eabihf`.

## Building

With the nRF Connect SDK (the version in `west.yml`) and the Zephyr SDK's
ARM toolchain installed, from the SDK's workspace:

```sh
rustup target add thumbv8m.main-none-eabihf
west build -b nrf54l15dk/nrf54l15/cpuapp path/to/smokebomb/packages/firmware/zephyr
```

CI's Zephyr image workflow (`.github/workflows/ci-zephyr.yml`) does the
same, and reports the image's RAM and flash in its summary.

**The board HAL is partly written** (`packages/firmware/hal/nrf54l15`). The
panels, IMU and touch have drivers. The rest are stubs that fail through
the shim (`sb_hal_todo`) so the whole firmware is still linked and the
image's memory figures are real. The firmware stops at the first stub it
calls at boot, and `main()` prints `smokebomb_main returned -1`.

## Bench bring-up

Built with `bringup.conf`, the image checks the bench's wiring instead of
running the firmware. It lists what answers on the I2C bus and which
drivers came up, then cycles test patterns across the panels one at a time
while streaming the IMU and touch readings. `BENCH.md` has the wiring and
how to read the output.

```sh
west build -b nrf54l15dk/nrf54l15/cpuapp path/to/smokebomb/packages/firmware/zephyr -- -DEXTRA_CONF_FILE=bringup.conf
west flash
```

CI uploads it as `bringup-nrf54l15dk`.

## The frame benchmark

Built with `bench.conf`, the image times a frame's heaviest work instead of
running the firmware: sugar in the air and landing, the pigs tumbling and at
rest, and dithering all six faces. It needs no drivers, so it runs on a bare
nRF54L15 DK:

```sh
west build -b nrf54l15dk/nrf54l15/cpuapp path/to/smokebomb/packages/firmware/zephyr -- -DEXTRA_CONF_FILE=bench.conf
west flash
```

It prints to the DK's serial console (115200 baud), a line per piece of
work with its time a frame and its share of a 60 Hz frame. CI uploads the
built image as `bench-nrf54l15dk`. It has no asset pack, so the landing's
embers (drawn with the pack's sprites) aren't drawn.
