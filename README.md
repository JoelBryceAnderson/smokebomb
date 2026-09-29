# Smokebomb

A smart die with six OLED faces, pre-rendered smoke effects, and rolls signed
by a secure element. This monorepo holds the firmware, a desktop simulator, the
backend, and the phone app.

| Package | What | Stack |
|---|---|---|
| [`packages/firmware`](packages/firmware) | Firmware core + HAL, built for the simulator or the nRF54L15 | Rust (`no_std`) |
| [`packages/simulator`](packages/simulator) | Runs the real firmware core in a browser | Rust/Axum + React/three.js |
| [`packages/server`](packages/server) | Device registry, roll verification, accounts, theme store | Rust/Axum + PostgreSQL |
| [`packages/mobile`](packages/mobile) | Phone app | Compose Multiplatform (iOS first) |
| [`packages/shared`](packages/shared) | Roll record, wire types, asset pack format | Rust (`no_std`) |

## Quick start

```sh
npm install
npm run build -w @smokebomb/simulator-web-ui
cargo run --features=simulator     # → http://localhost:3000, click "Throw"
```

## Docs

- [Architecture](docs/ARCHITECTURE.md): how the pieces fit, the roll signing format, the animation pack layout
- [Getting started](docs/GETTING_STARTED.md): building and running each piece
- [API](docs/API.md): REST endpoints
- [Simulator spec](docs/SIM_SPEC.md): how the die must look and behave, 1:1 with the interactive mockup

## How it has evolved

The repo started as a scaffold: all five packages existed, but only as
skeletons. Since then the work has gone into making the firmware core real and
provable in the simulator, in this order:

1. **Scaffold**: the monorepo layout, firmware core + HAL, simulator, server
   schema and mobile shells.
2. **Simulator you can drive**: reachable from an iPad over the local network,
   with a spec ([SIM_SPEC](docs/SIM_SPEC.md)) corrected against the interactive
   mockup and its decisions (H1 to H7) recorded, and a check that catches a
   stale web UI build.
3. **Rendering and rolls**: roll format v2, the renderer (fonts, primitives,
   boot, wake label, result screens), the simulator's world model, and golden
   frames captured from the mockup that the firmware is tested against.
4. **The menu**: hold to open, tip to navigate, hold to save, then multi-turn
   swipes that cross several screens at once.
5. **Smoke**: the live particle system.
6. **Polish and fixes**: screens keep their orientation while a result is up
   (H9), the menu always lands on a face that is in front, and a clock-change
   panic is fixed.

Still to come: real nRF54L15 drivers and DFU, BLE message framing, persisting
settings to flash, server auth and roll storage, and real BLE/NFC in the phone
app. The full list is at the end of the
[architecture doc](docs/ARCHITECTURE.md#not-yet-built).
