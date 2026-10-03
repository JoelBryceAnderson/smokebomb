# The bench

The bench is an nRF54L15 DK wired to six SSD1317 panels and a STEMMA QT
chain of Adafruit breakouts: the LSM6DSOX IMU, the CAP1188 touch
controller, the DRV2605L haptic driver and a magnetometer (MMC5603 or
LIS2MDL). This page is the wiring table. The devicetree overlay
(`boards/nrf54l15dk_nrf54l15_cpuapp.overlay`) says the same thing in code,
and CI builds it, so keep the two in step.

## Before wiring

- **Set the DK's I/O voltage to 3.3 V** in nRF Connect's Board Configurator.
  The DK can run its GPIOs at 1.8 V. The breakouts pull SDA and SCL up to
  their own 3.3 V, which is too much for 1.8 V pins.
- Power the breakouts' VIN from the DK's VDD (3.3 V) and share ground.
- The panels need their own 12 V VCC and the support parts from their
  datasheet (IREF resistor, capacitors). This page only covers their logic
  signals.

## Pin map

| DK pin | Signal | Goes to | Why this pin |
|---|---|---|---|
| P2.06 | SPI SCK | every panel's SCLK (D0) | SPIM00 is the only SPIM that runs faster than 8 MHz, and its clock has to be on P2. Nordic's SPIM test uses this pin on the DK |
| P2.08 | SPI MOSI | every panel's SDIN (D1) | SPIM00 data, on P2 too. The panels only receive, so there's no MISO |
| P1.09 | D/C | every panel's D/C | Button 1's pin. Don't press it |
| P1.08 | RESET (active low) | every panel's RES# | Button 2's pin. Don't press it |
| P2.10 | CS +X (face 0) | panel +X CS# | free |
| P2.09 | CS −X (face 1) | panel −X CS# | LED 0's pin. The LED lights while the panel is idle |
| P2.07 | CS +Y (face 2) | panel +Y CS# | LED 2's pin |
| P1.10 | CS −Y (face 3) | panel −Y CS# | LED 1's pin |
| P1.14 | CS +Z (face 4) | panel +Z CS# | LED 3's pin |
| P1.13 | CS −Z (face 5) | panel −Z CS# | Button 0's pin. Don't press it |
| P1.11 | I2C SDA | STEMMA QT SDA (blue) | TWIM22. Nordic's DK shields use this pair for I2C |
| P1.12 | I2C SCL | STEMMA QT SCL (yellow) | TWIM22 |
| P0.04 | IMU INT1 | LSM6DSOX INT1 | Button 3's pin. Wired for later: the driver polls for now |
| VDD | 3.3 V | STEMMA QT VIN (red) | |
| GND | ground | STEMMA QT GND (black), panels' VSS | |

The overlay takes the DK's LEDs and buttons for chip-selects and control
lines, and deletes their devicetree nodes so nothing else drives those
pins. SPIM00's default pins (P2.01, P2.02, P2.04, P2.05) go to the DK's
onboard flash. The panels use SPIM00 on other pins, so that flash isn't
reachable in these builds.

Panel N is `Face` index N: +X, −X, +Y, −Y, +Z, −Z.

### The I2C chain

| Part | Board | Address | Zephyr driver | Notes |
|---|---|---|---|---|
| LSM6DSOX | Adafruit | 0x6A | `st,lsm6dso` (same registers, WHO_AM_I 0x6C) | ±16 g, ±2000 dps, 208 Hz. If its address jumper is cut it answers at 0x6B instead: change `reg` in the overlay |
| CAP1188 | Adafruit | 0x29 | `microchip,cap12xx` | Adafruit's older CAP1188 board has no STEMMA QT socket: if yours doesn't, wire SDA, SCL, VIN and GND by hand. Leave AD and RST unconnected. Pads C1–C6 are faces +X…−Z. Polled every 10 ms |
| DRV2605L | Adafruit | 0x5A | `ti,drv2605` | LRA mode. Wire the LRA to its motor terminals |
| MMC5603 | Adafruit | 0x30 | `memsic,mmc56x3` | one-shot mode |
| LIS2MDL | Adafruit | 0x1E | `st,lis2mdl` | the other magnetometer candidate. The overlay lists both; whichever isn't on the chain fails its init and the rest carry on |

Zephyr already has drivers for all four. The CAP12xx driver was written for
the CAP1203, CAP1206, CAP1293 and CAP1298. It doesn't check the product ID,
and the CAP1188 uses the same main control, input status and sensitivity
registers. But it also writes the signal-guard (0x29) and calibration
sensitivity (0x80–0x81) registers, which the CAP1188 may not have. If
touches don't register on the bench, a small CAP1188 driver of our own is
the fix.

## SPI on paper

What the panels need from the bus decides which SPIM to use and how fast to
run it.

- **The SPIMs.** SPIM00 runs at up to 32 MHz, with its clock and data on
  P2. SPIM20, SPIM21, SPIM22 and SPIM30 top out at 8 MHz. (SPIM20 also
  shares its registers with UARTE20, the DK's console.)
- **The panels' limit.** The SSD1306-family SPI cycle time is 100 ns, so
  10 MHz at most. u8g2 drives the SSD1317 at 8 MHz. That ceiling applies on
  every SPIM, so the overlay runs the panels at 8 MHz.
- **What the SSD1317 actually needs.** It is a 1-bit controller
  ([below](#the-ssd1317-is-one-bit)): 1152 bytes a frame, plus 3 address
  bytes for each of the 12 pages. That's 1188 B × 6 faces × 60 Hz =
  3.4 Mbit/s, 43% of an 8 MHz bus. On top of that come the gaps around
  each transfer: 24 per face per frame, 8640 a second. Writing a whole face
  in horizontal addressing mode would make that 2 per face; it's worth
  doing once the page writes are proven on glass.
- **What 4bpp would need.** 4608 B × 6 × 60 = 13.3 Mbit/s before overhead,
  more than a 10 MHz panel clock allows on one bus. A grey panel would need
  two buses (SPIM00 for three faces and SPIM21 for the other three, each
  6.6 Mbit/s, 83% of 8 MHz). Or it would need to resend only the faces that
  changed.

**Decision:** the panels go on SPIM00. At 1 bit a pixel any SPIM could
carry them, but SPIM00 leaves headroom up to the panels' own 10 MHz, and
it's the instance a second bus would be split from.

**For the product board:** the nRF54L15 has no QSPI peripheral. Nordic
drives QSPI flash from the FLPR core (sQSPI) on P2.00–P2.05, the pins the
DK's flash uses. If the 64 MB asset flash goes there, P2 has 5 pins left
for the panels' SCK and MOSI. Check that before laying out the board.

## The SSD1317 is one-bit

The panel is 96×96. Its controller, the SSD1317, can drive up to 128×96,
and the panel is wired to 96 of its 128 columns (16–111, the driver's
`COLUMN_OFFSET`). Solomon lists the SSD1317 as a **monochrome**
controller, and u8g2's SSD1317 96×96 driver treats it as one:
SSD1306-style pages, a bit a pixel. The firmware draws 16 grey levels (`FRAME_BYTES` = 4608, SIM_SPEC's "16 gray
levels" for the ER-OLED0.96-6W).

Until that's settled, the driver (`hal/nrf54l15/src/ssd1317.rs`) reduces
each 4bpp frame to 1 bit, lighting levels 8 and up, so bring-up can light
glass either way. Check the datasheet's command table, then pick one: a
grey controller for 96×96 (the SSD1327 drives 16 levels, and 96×96 SSD1327
modules exist), or design the faces for one bit.

## Bring-up

The bring-up image checks the wiring before the firmware does anything
else:

```sh
west build -b nrf54l15dk/nrf54l15/cpuapp path/to/smokebomb/packages/firmware/zephyr -- -DEXTRA_CONF_FILE=bringup.conf
west flash
```

On the DK's serial console (115200 baud) it prints, in order:

1. **The I2C scan.** Every address that answers, named if it's a part the
   bench expects, with its ID register checked (`id ok`, `WRONG ID`). Each
   expected part that doesn't answer is listed as `MISSING`.
2. **Each Zephyr driver's state**: `ready`, or `FAILED its init` (Zephyr's
   log line just above says why).
3. **A haptic click**, and whether it was sent.
4. **The panels**, reset and set up.

Then it loops. Each face in turn shows three patterns for a second each:

- **Which face, and which way up.** A border, a block in the top-left
  corner, and one bar for face 0 (+X) up to six for face 5 (−Z).
- **Every pixel lit.**
- **A checkerboard** of 8-pixel squares.

Every 100 ms it prints the IMU (milli-g, milli-degrees a second) and the
touch mask.

What it tells you:

| You see | It's likely |
|---|---|
| A part `MISSING` in the scan | wiring: power, SDA/SCL swapped, or a STEMMA cable not seated |
| A part answers but its driver `FAILED its init` | code or the overlay: a wrong address, or Zephyr's driver and the part disagree |
| No panel lights, every write `Ok` | the panel's 12 V supply or RESET, or the init sequence |
| One panel stays dark | its CS wire |
| A face shows another face's bars | CS wires swapped |
| The corner block isn't top left | the remap (`0xA0`/`0xC8` in the init sequence) |
| The image is offset or wraps | `COLUMN_OFFSET` |
| IMU axes don't match the die's | the breakout's orientation on the bench. The HAL takes its axes as the die's |
