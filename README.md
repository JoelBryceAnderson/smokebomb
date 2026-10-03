# Sugarcube

Sugarcube is a smart die with six 96×96 OLED faces. Throw it and a storm of sugar
crystals plays across the screens, then the result shows on every face. Each roll
is signed by a secure element inside the die, so a phone app and backend can
prove that a roll really came from that die and wasn't edited or replayed. That
is what makes it usable for organised play, where rolls have to be trusted.

This monorepo holds everything: the firmware, a desktop simulator that runs
the real firmware, the backend that verifies rolls, and the phone app.

## What it does

- **Rolls and shows the result.** Shake, throw, land. The firmware reads the
  gyro, works out the landing face and shows the result on every screen except
  the one facing down. Screens keep their orientation while a result is up.
- **Live sugar.** A particle simulation in firmware reacts to gravity,
  shaking and the landing face: sugar crystals bank up while you shake, spray
  on the throw and tumble off the faces as the result appears. Themes ship
  sprites and parameters, not frames.
- **Game modes.** Dice, Pass the Pot (the menu page is the bills in your
  hand) and Hot Potato (set a fuse length and pass it round),
  and Pig Toss (two 3D pigs tumble across the screens instead of sugar and land
  in scored poses; set 2–6 players, tap to bank and pass the die).
- **On-die menu.** Press and hold a screen to open it, tip the die to move
  between pages and values, tap to edit a setting, hold to save. Turning
  several faces in one swipe steps several pages or values. Settings include
  sugar, sleep timeout and power off.
- **Signed, chained rolls.** Every roll carries a counter, the dice, a digest,
  the previous roll's hash, and a P-256 signature from the secure element.
- **Verification.** The server checks signatures against a registered device
  key and rejects duplicate counters and digests.
- **Themes.** Asset packs (sprites, fonts, parameters) that live in the die's
  QSPI flash.

## Status

Where each area stands. Update this table when something changes.

| Area | Status | Notes |
|---|---|---|
| Firmware core (state machine, motion, rolls, menu, renderer, smoke) | ✅ Working | Runs on the simulator HAL; screens are covered by snapshot tests |
| Simulator (browser, iPad over the LAN) | ✅ Working | Server owns the die's physics; the browser draws it |
| Shared roll format and wire types | ✅ Working | Roll format v2; one definition used by firmware and server |
| Server: device registry, signature verification, schema | ✅ Working | Axum + PostgreSQL |
| Server: accounts and auth, roll storage, sessions, theme purchase | 🧪 Stubbed | Endpoints exist; see [API](docs/API.md) |
| nRF54L15 hardware drivers, Zephyr shell, MCUboot and DFU | 🗓 Planned | The board HAL is a stub |
| BLE message framing, settings and roll chain persisted to flash | 🗓 Planned | Types are defined, not yet serialised |
| Phone app: navigation and screens | 🧪 Stubbed | Drives the simulator over the network; real BLE, NFC and the API client are not wired up |
| Store: modes and themes | 🧪 Started | Die offers licensed + enabled modes; app has mode switches. See [Store](docs/STORE.md) |
| 30 mm die, 64×64 colour panels | 🧪 Proof of concept | Builds and runs in the simulator; SSD1357 driver from its datasheet, module wiring TBD. See [30 mm die](docs/30mm/README.md) |

The full list of gaps is at the end of the
[architecture doc](docs/ARCHITECTURE.md#not-yet-built).

## How it fits together

```text
        shared (no_std): roll record, wire types, asset packs
          ▲               ▲                    ▲
   firmware core ──► HAL traits            server (Axum + Postgres)
     ▲        ▲          ▲   ▲                  ▲
simulator   nRF54L15   (BLE / NFC)         phone app
```

The firmware's application logic is Rust behind hardware traits, so the same
code runs on the die and in the simulator. Zephyr is only the RTOS shell.

| Package | What | Stack |
|---|---|---|
| [`packages/firmware`](packages/firmware) | Firmware core + HAL, built for the simulator or the nRF54L15 | Rust (`no_std`) |
| [`packages/simulator`](packages/simulator) | Runs the real firmware core in a browser | Rust/Axum + React/three.js |
| [`packages/server`](packages/server) | Device registry, roll verification, accounts, theme store | Rust/Axum + PostgreSQL |
| [`packages/mobile`](packages/mobile) | Phone app | Compose Multiplatform (iOS first) |
| [`packages/shared`](packages/shared) | Roll record, wire types, asset pack format | Rust (`no_std`) |

## Try it

```sh
npm install
npm run build -w @smokebomb/simulator-web-ui
cargo run --features=simulator     # → http://localhost:3000, hold "Throw", release
```

Hold a screen for the menu. To drive it from an iPad, the server, the
database and the tests, see [Getting started](docs/GETTING_STARTED.md).

## 30 mm die: 64×64 colour proof of concept

The firmware builds for two dice from one codebase. The display target is
picked at compile time (`smokebomb_hal::target`):

| Target | Die | Panels |
|---|---|---|
| `TARGET_34_GREY96` (default) | 34 mm | six SSD1317 96×96, 16 grey levels |
| `TARGET_30_RGB64` | 30 mm | six 0.6" 64×64 RGB (SSD1357), RGB565 |

The 64×64 screens are their own layouts, not the 96×96 ones scaled down:
bitmap fonts, condensed numerals and a small palette in which colour
carries meaning. The 34 mm die is unchanged: its snapshot sheets
regenerate byte for byte.

```sh
cargo fw                  # 34 mm image (as before)
cargo fw30                # 30 mm image: --features target-30-rgb64
cargo sim -- --die 30     # simulator on the 30 mm die (or SMOKEBOMB_DIE=30);
                          # or switch in the page's Die section
UPDATE_SNAPSHOTS=1 cargo test -p smokebomb-core --test contact_sheet  # regenerate the contact sheets
```

What's done and what isn't:
- **Driver.** The SSD1357 driver's commands, power sequence and timing come
  from the datasheet, and its command stream is tested. The board's SPI
  bus is a stub, like the other drivers.
- **Placeholders.** Values that depend on the NHD-0.6-6464G module are
  marked `MODULE TODO`: which SEG/COM lines it uses, remap, VCC and tuning.
- **Estimates.** The lit-area, window and mask sizes and the number of SPI
  buses are estimates.
- **Fallback screens.** Pig Toss, Hot Potato, Pass the Pot's bills and the
  Nest's clock fall back to the 96×96 layout.

Findings, contact sheets and the full list of what's stubbed:
[docs/30mm](docs/30mm/README.md).

## Docs

- [Architecture](docs/ARCHITECTURE.md): decisions, repo map, roll signing format, animation packs, what isn't built
- [Getting started](docs/GETTING_STARTED.md): building and running each piece
- [API](docs/API.md): REST endpoints and their status
- [Simulator spec](docs/SIM_SPEC.md): how the die looks and behaves, which began as a 1:1 copy of the interactive mockup
- [Store](docs/STORE.md): selling modes and themes, licenses, choosing a die's modes from the app
- [30 mm die](docs/30mm/README.md): the 64×64 colour proof of concept, its contact sheets and findings
