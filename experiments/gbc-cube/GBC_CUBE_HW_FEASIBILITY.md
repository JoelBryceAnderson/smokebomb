# GBC cube on real hardware: feasibility

Can Pokémon Crystal, wrapped round the cube as in the simulator, run on the
Gen 1 board: an nRF54L15 (Cortex-M33 at 128 MHz, 256 KB RAM, about 1.5 MB of
internal non-volatile memory), 64 MB of external flash and six 64×64 colour
OLEDs?

**Verdict: it runs with frameskip.** The game itself can run at full speed.
The faces update at 30 fps while walking (60 fps if Crystal turns out to be
light on CPU). Five faces at 60 fps doesn't fit. RAM is the tightest
budget, and the ROM has to live mostly in internal RRAM, because external
flash can't be executed or read in place. Three things decide where in that
range it lands, and each takes one measurement with the real ROM or a DK
(see [What's still unknown](#whats-still-unknown)).

How sure each number is:

| Kind | Meaning |
|---|---|
| **Measured** | Run in this repo: the simulator on the host, or the Cortex-M33 build under QEMU |
| **From Nordic** | Nordic-maintained sources, linked at the end |
| **Estimate** | Reasoned, not measured: a range, and the assumption behind it |

## CPU

### How it was measured

`m33bench/` builds walnut-cgb and the cube's core for
`thumbv8m.main-none-eabihf`, the nRF54L15's core, with the same compiler
flags a firmware build would use. It runs them on QEMU's `mps2-an505`
Cortex-M33 board with `-icount shift=0`, where every executed instruction
advances virtual time by 1 ns. SysTick reads that clock, calibrated by a
loop of known length (50.0 instructions per tick).

That counts **instructions, not cycles**: QEMU doesn't model the pipeline,
the cache or RRAM wait states. Cycles are instructions × CPI. For a
branchy C interpreter on a single-issue Cortex-M33 this note assumes a CPI
of **1.25–1.6** (estimate). A DK run fixes it ([Optional bench run](#optional-bench-run-on-a-dk)).

The frame budget at 128 MHz is 128 MHz ÷ 59.73 Hz = **2.14 M cycles**.

Workloads:

- **Emulator, CPU bound:** blargg's `cpu_instrs` test ROM. It never lets
  the CPU idle, so it's the worst case for interpretation. It's a DMG ROM,
  and walnut-cgb switches speed on its `STOP`s, so some of its frames ran
  in double speed, which gives a figure for both speeds.
- **Emulator, CPU idle:** the demo cart's ROM, which sits in `halt` with
  the LCD on.
- **Renderer:** RAM snapshots of the demo cart (`gbc-cube snapshot`),
  mid-step in the overworld and with a text box open. The demo writes
  Crystal's RAM layout, and the renderer is checked pixel for pixel against
  the PPU on it (`demo/tests/renderer_matches_ppu.rs`).

Crystal itself wasn't available here: no ROM goes in the repo or the build
container. The same benchmark runs on a ROM and its save:

```sh
GBC_CUBE_M33_ROM=/path/crystal.gbc GBC_CUBE_M33_SAV=/path/crystal.sav \
GBC_CUBE_M33_SNAPSHOT=/path/snap cargo run --release   # in m33bench/
```

### Results (measured, Cortex-M33 instructions per frame)

| What | Instructions/frame |
|---|---|
| Emulator, CPU idle, LCD drawing lines | 1.08 M |
| Emulator, CPU idle, line drawing off | 0.064 M |
| …so drawing 144 lines costs | **1.01 M** |
| Emulator, CPU bound, single speed (247 frames) | 2.13 M mean, 2.90 M max |
| Emulator, CPU bound, double speed (353 frames) | 3.02 M mean, 3.57 M max |
| …so the game's CPU at 100 %, single speed, adds about | **1.05 M** |
| …and at 100 % in double speed about | **2.0 M** |
| Phase 3: `world::draw` (192×192 canvas + objects) | 0.39 M |
| Fog light grid | 0.03 M |
| Drape the canvas over 5 faces | 0.59 M |
| Screen classification (`wTilemap` against the map) | 0.04 M |
| Phase 2: canvas from the emulator's frame | 0.22 M |
| C: the text box re-flowed onto the front face | 0.09 M |
| **`Cube::render`, overworld, all of it** | **1.09 M** |

The renderer's first version took 2.61 M. Two changes brought it to 1.09 M:
decoding tile rows 8 pixels at a time from a lookup table, and draping each
face as an affine walk that skips the fog maths outside the fog. The PPU
comparison test still passes on the faster version. There's more to take:
the drape is still about 29 instructions a pixel.

Host numbers for comparison (measured, one core of a 2.8 GHz Xeon, from
`gbc-cube bench`): emulator 254 µs a frame on `cpu_instrs` and 74 µs on the
demo; cube 58 µs. One host µs is about 10,400 M33 instructions of emulator
work, but only about 18,800 of renderer work (the x86 does more per
instruction there). So run `m33bench` rather than scaling host times.

### Double speed

Crystal's start-up calls `NormalSpeed` (`home/init.asm` in pret/pokecrystal),
so the game runs in single speed. `DoubleSpeed` exists (`home/double_speed.asm`),
but I couldn't search the disassembly for its callers from here. The
simulator counts double-speed frames (`gbc-cube bench --rom`, the
`double_speed_frames` field), so a run with the ROM settles whether any part
of normal play switches. If something does, its CPU cost roughly doubles
(above: 2.0 M instead of 1.05 M at full load).

### What fits

The game's CPU load L is the fraction of each frame Crystal isn't halted in
`DelayFrame`; it's unknown until the ROM runs. On the die the emulator only
needs to draw lines when a screen copies the frame (`Report::needs_frame`):
the world (Phase 3) and C's text panel are drawn from RAM and VRAM.

| Situation | Instructions/frame | Cycles at CPI 1.25 | at CPI 1.6 |
|---|---|---|---|
| Overworld, Phase 3, lines off, faces every frame | 1.15 M + 1.05 M·L | L ≤ 0.53 fits | L ≤ 0.18 fits |
| Same, faces every 2nd frame (30 fps) | 0.61 M + 1.05 M·L | any L fits | L ≤ 0.70 fits |
| Overworld, Phase 2 (frame copy), every frame | 1.96 M + 1.05 M·L | never | never |
| Text or menu over the map (C) | about 1.25 M + 1.05 M·L | L ≤ 0.45 fits | L ≤ 0.09 fits |
| Battle (C: frame crops + text), lines every 2nd frame | about 0.72 M + 1.05 M·L | L ≤ 0.95 fits | L ≤ 0.59 fits |

"Fits" means it stays within the 2.14 M cycles. When it doesn't, the
emulator still runs every frame and the cube draws every second or third
frame: the game keeps its speed and the faces drop to 30 or 20 fps.

Two things are visible here. Phase 3 is cheaper on the die than Phase 2,
because drawing the world from RAM lets the emulator skip its own line
rendering. And the emulator, at about 1 M per frame of CPU at full load plus
the PPU when needed, fits on its own; it's the 1.09 M of cube rendering on
top that needs frameskip unless Crystal is light.

**Code and cache (measured / estimate).** walnut-cgb's hot code is small:
`__gb_step_cpu` is 17.0 KB of Thumb-2, `__gb_draw_line` and `__gb_write`
2 KB each. Whether that runs from RRAM at full speed depends on the
nRF54L15's instruction cache, whose size and RRAM wait states I couldn't
check here (**Datasheet TODO**). If it thrashes, copying the interpreter to
SRAM costs about 21 KB of RAM.

## RAM

256 KB of SRAM (from Nordic: `nrf54l15.dtsi`). Measured sizes from the
M33 build, with the simulator-only parts left out:

| What | Bytes | Notes |
|---|---|---|
| walnut-cgb state: WRAM 32 KB, VRAM 16 KB, OAM, I/O, CGB palettes | ≈50,000 | The context is 121,264 B in the sim build. That includes an RGB565 frame (46,080), an index frame (23,040) and ROM statistics (2,144), none of which the die needs. |
| Cartridge RAM | 32,768 | Crystal has four 8 KB banks; written back to flash on save |
| The PPU frame as palette indexes | 23,040 | Only for Phase 2, A, B and battle crops. Without them it can go. |
| Cube canvas (192×192 indexes, light grid) | 38,193 | |
| Metatile cache | ≈4,230 | |
| Fallback state | 360 | Generic A on other games adds a 23,040 B previous frame |
| Face buffers | 16,384 | Two 8 KB buffers, filled and sent face by face. Five whole faces would be 40,960. |
| ROM page cache | 16,384 | Four 4 KB pages (see below) |
| Zephyr, BLE, drivers, stacks | 50–70 K | **Estimate.** The 30 mm dice image's statics are 100 KB with all its game modes, the GBC image needs fewer |
| **Total** | **≈230–250 K** | of 256 K |

It fits, just, if faces are streamed rather than all held, and with no
room for a bigger ROM cache. Options if it doesn't:

- Drop the frame (23 KB) and run A/B/battle on fewer faces.
- Draw faces straight from the map without the 192×192 canvas (37 KB),
  which costs more CPU per pixel.
- Keep cart RAM in RRAM, only if RRAM write endurance allows Crystal's SRAM
  writes (**Datasheet TODO**).

## ROM access

Pokémon Crystal is 2 MB: 128 banks of 16 KB.

**Can the nRF54L15 run or read external flash in place? No.** From Nordic:

- The nRF54L15's devicetree (Zephyr, maintained by Nordic) has no QSPI
  peripheral and no memory-mapped external-flash region.
- Its serial peripherals are SPIM00 at up to 32 MHz and SPIM20/21/22/30 at
  up to 8 MHz.
- The nRF Connect SDK's *QSPI XIP split image* documentation says "This
  feature is supported on nRF5340".
- On the nRF54L15 DK, the 8 MB MX25R6435F external flash hangs off SPIM00,
  configured at 8 MHz.
- Nordic's forum describes a software QSPI ("sQSPI") for the nRF54L15 that
  runs on the FLPR RISC-V core. I could only see search snippets of that
  thread from here, not the thread itself; its throughput is unknown.

So every byte of ROM outside internal memory has to be copied into RAM
before the emulator reads it.

**Design: hot banks in RRAM, the rest paged.**

1. **Internal RRAM** is 1524 KB (from Nordic) and memory-mapped. After the
   firmware (estimate 200–300 KB) it holds about 1.2 MB, which is 75 of
   Crystal's 128 banks.
   - Put bank 0 and the most-read banks there. `gbc-cube bench --rom` counts
     reads per bank and prints how many banks cover 50, 90 and 99 % of
     reads, plus the hottest banks. That run picks the set.
   - walnut-cgb reads ROM through a callback (`csrc/shim.c`), so the
     resident check is a table lookup on the bank number.
2. **External flash** holds the whole ROM. The other banks are paged into
   a small RAM cache: four 4 KB pages, least recently used.
3. **Miss cost (estimate from bus rates).**
   - 4 KB over single-bit SPI at 32 MHz (SPIM00) takes 1.0 ms, about 6 %
     of a frame. At 8 MHz it takes 4.1 ms, a quarter of a frame.
   - A whole 16 KB bank would take 4.1 ms or 16.4 ms, up to a full frame.
     So the page cache uses pages, not whole banks.
   - The CPU stalls on a miss: the game only writes the MBC bank register a
     few instructions before it reads from the new bank, so there's no
     prefetch window to speak of.
4. **Bank switch rate (measured, test ROM only).** `cpu_instrs` changes
   bank 30 times a frame, between bank 0 and one other, both resident.
   Crystal switches far more often (it `farcall`s constantly), which is why
   its working set must sit in RRAM. Its real rate comes from
   `gbc-cube bench --rom`.

## Display bandwidth

The planned bus (from the repo's 30 mm findings, `docs/30mm/README.md`):

- The SSD1357's 4-wire SPI tops out at 10 MHz.
- The board estimate assumes 85 % bus efficiency and two buses.
- The nRF54L15's other SPI instances run at 8 MHz; SPIM00 runs at 10 MHz
  for a panel, but it's the one the external flash wants.

| | Bytes/frame | at 60 fps | at 30 fps |
|---|---|---|---|
| 5 faces × 64×64 × 2 B | 40,960 | 2.46 MB/s | 1.23 MB/s |
| One 8 MHz bus at 85 % | | 0.85 MB/s | |
| Buses needed | | **3** | **2** |

While walking, the map scrolls under every face, so every pixel of every
face changes each frame. The firmware's changed-tile updates don't help
mid-step. Standing still, only animated water and flowers and moving people
change, and dirty tiles make that nearly free.

So, consistent with the CPU budget: **faces at 30 fps on two buses, 60 fps
would need three**, or one bus at about 17 fps.

One thing worth checking: the SSD1357 has 128×128 of display RAM behind a
64×64 panel, and a display start line command (in the repo's driver). That
could make vertical scrolling a few command bytes instead of a full frame,
for the faces where the map scrolls along the panel's columns.
**Datasheet TODO:** horizontal scrolling and the module's mapping.

## Power and battery

Rough, and mostly estimates: there's no 30 mm power budget in the repo yet.

| Load | Estimate | Note |
|---|---|---|
| nRF54L15 running flat out at 128 MHz | 3–5 mA | **Datasheet TODO** |
| Five PMOLED panels showing a bright game | 75–150 mA | 15–30 mA each at moderate brightness, an assumption (NHD-0.6-6464G datasheet TODO). Game Boy maps are mostly light colours and text boxes are white, which is close to an OLED's worst case. |
| SPI transfers, flash paging, IMU | 3–8 mA | |
| **Total** | **≈80–160 mA** | |

The panels dominate. A 30 mm die might hold a 100–200 mAh cell (an
assumption), which gives about an hour or two of play. Ways to stretch it:

- dim the side faces and keep the up face bright;
- darken the palette (a "night" filter);
- keep the bottom face off (it is already black);
- sleep the panels when the cube sits still on the menu.

## Verdict

**Runs with frameskip.**

- The game runs at full speed. On the CPU's single-speed path the
  emulator costs 0.06–1.1 M instructions a frame, plus 1 M when a screen
  needs the PPU's pixels.
- The faces update at 30 fps while walking, which the SPI bandwidth
  (two buses) and the CPU budget both point to.
- It reaches 60 fps faces only if Crystal's overworld leaves the CPU idle
  half the frame or more and the CPI comes in near 1.25. It needs a third
  display bus either way.
- RAM fits with streamed faces.
- The ROM comes from RRAM-resident hot banks plus a small page cache over
  external flash.

**Gen 2 hardware** would change:

- **A part that runs external flash in place, with more RAM.** The nRF5340
  is the Nordic part the SDK documents for QSPI XIP; its application core is
  also a Cortex-M33 at 128 MHz, but with 512 KB of RAM. That would remove
  the page cache and the RAM squeeze, but not add CPU. A faster core (a
  higher-clocked nRF54H-class part, **to verify**) or PSRAM for ROM and
  canvases would give both.
- **Display bandwidth:** three or more SPI buses, faster panel interfaces,
  or panels with hardware scrolling in both axes.
- **A bigger battery, or panels that are cheaper to light.**

## What's still unknown

All of these are one measurement away.

1. **Crystal's CPU load L, and whether it ever uses double speed.** Run
   `m33bench` with the ROM and a save that starts in the overworld, and
   `gbc-cube bench --rom` for `double_speed_frames`.
2. **CPI and cache behaviour on silicon.** See the next section.
3. **Bank coverage.** `gbc-cube bench --rom` prints how many banks cover
   90 and 99 % of reads. If 99 % of reads fit in about 70 banks, RRAM plus a
   page cache works as described.
4. **Datasheet TODOs above:** the RRAM wait states and cache size, the
   nRF54L15 run current, the sQSPI throughput, the SSD1357 scroll commands,
   and the panel current.

## Optional bench run on a DK

Not done: there was no hardware here, and nothing was flashed. The way to do
it:

1. Build `m33bench`'s workloads into a Zephyr app for `nrf54l15dk/nrf54l15/cpuapp`.
   - Replace QEMU's SysTick-under-icount timing with the DWT cycle counter
     (`DWT->CYCCNT`), which gives real cycles.
   - Print over the DK's UART instead of semihosting.
2. Run it from RRAM first, then with `__gb_step_cpu` placed in SRAM, to
   measure the cache's effect.
3. Divide cycles by the instruction counts above to get the real CPI.

## Sources

From Nordic, as published on GitHub (the container couldn't reach Nordic's
own sites):

- nRF54L15 RAM and RRAM sizes:
  [zephyr `dts/vendor/nordic/nrf54l15.dtsi`](https://github.com/zephyrproject-rtos/zephyr/blob/main/dts/vendor/nordic/nrf54l15.dtsi)
- The 128 MHz CPU clock (`hfpll`), SPIM instances and their maximum
  frequencies, and the absence of a QSPI peripheral:
  [zephyr `dts/vendor/nordic/nrf54l_05_10_15.dtsi`](https://github.com/zephyrproject-rtos/zephyr/blob/main/dts/vendor/nordic/nrf54l_05_10_15.dtsi)
- The DK's external flash (MX25R6435F on SPIM00 at 8 MHz):
  [zephyr `boards/nordic/nrf54l15dk/nrf54l_05_10_15_cpuapp_common.dtsi`](https://github.com/zephyrproject-rtos/zephyr/blob/main/boards/nordic/nrf54l15dk/nrf54l_05_10_15_cpuapp_common.dtsi)
- QSPI XIP supported on the nRF5340:
  [sdk-nrf `doc/nrf/app_dev/bootloaders_dfu/qspi_xip_split_image.rst`](https://github.com/nrfconnect/sdk-nrf/blob/main/doc/nrf/app_dev/bootloaders_dfu/qspi_xip_split_image.rst)
- FLPR code runs from RRAM:
  [zephyr `snippets/nordic/nordic-flpr-xip/README.rst`](https://github.com/zephyrproject-rtos/zephyr/blob/main/snippets/nordic/nordic-flpr-xip/README.rst)
- Nordic DevZone, seen only as search snippets:
  [use code sections in the external flash](https://devzone.nordicsemi.com/f/nordic-q-a/121948/use-code-sections-in-the-external-flash/540206),
  [nRF54L15: 32 MHz dual QSPI/SPI devices](https://devzone.nordicsemi.com/f/nordic-q-a/119416/nrf54l15-32-mhz-dual-qspi-spi-devices/525326),
  [nRF54L15 QSPI maximum speed](https://devzone.nordicsemi.com/f/nordic-q-a/122838/nrf54l15-qspi-maximum-speed)

Game side: [pret/pokecrystal](https://github.com/pret/pokecrystal)
(`home/init.asm`, `home/double_speed.asm`, and the symbol files on its
`symbols` branch).

Display: the repo's [30 mm findings](../../docs/30mm/README.md) and the
SSD1357 driver (`packages/firmware/hal/nrf54l15/src/ssd1357.rs`).
