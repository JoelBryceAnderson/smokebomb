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

This is a scaffold. Anything not built yet is listed at the end of the architecture doc.
