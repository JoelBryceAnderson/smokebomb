# Getting started

## Prerequisites

| Tool | Version | Needed for |
|---|---|---|
| Rust (rustup) | stable ≥ 1.85 | firmware, simulator, server |
| `thumbv8m.main-none-eabihf` target | — | nRF54L15 builds: `rustup target add thumbv8m.main-none-eabihf` |
| Node.js | ≥ 20 (22 recommended) | Nx, simulator web UI |
| Docker | any | local PostgreSQL, or install PostgreSQL 16 yourself |
| JDK | 17+ | mobile |
| Android SDK | API 36 | Android app (Android Studio installs it) |
| Xcode + XcodeGen | Xcode 16+, `brew install xcodegen` | iOS app |

Then, from the repo root:

```sh
npm install          # Nx + web UI dependencies
cargo build          # every Rust crate except the backend server
```

## Simulator (fastest way to see the firmware run)

```sh
npm run build -w @smokebomb/simulator-web-ui   # once, and after UI changes
cargo run --features=simulator                 # → http://localhost:3000
```

Or run both with Nx: `npx nx run simulator-server:serve`.

In the browser:

- **Throw** plays a scripted pick-up, shake, throw and landing. The die rolls,
  signs the roll, and shows the result on the up face. The roll appears in the
  **Signed rolls** list with its digest and signature.
- **Pick up** and **Shake** send partial gestures. A pick-up while in the menu
  turns the page.
- **Face up** and **Tilt** set the resting orientation, which becomes the
  accelerometer reading.
- **Press and hold a face** for 1.5 s to open the menu. A short tap changes the
  value on the current page; hold again to save and leave.
- **On charging nest** switches to the nest screen.

To work on the UI with hot reload, keep the simulator running and in a second
terminal run:

```sh
npm run dev -w @smokebomb/simulator-web-ui     # → http://localhost:5173 (proxies to :3000)
```

### From an iPad (or any other device on your network)

The simulator server has to run on a Mac or PC; the iPad is just the screen and
controls. Bind to all interfaces and open the page from Safari:

```sh
HOST=0.0.0.0 cargo run --features=simulator
# on the iPad: http://<your-mac>.local:3000  (or the Mac's IP address)
```

Drag to orbit, tap and hold a face to touch it, and use the panel for throw
and tilt. The simulator has no authentication, so only do this on a network you
trust. The default (`HOST` unset) accepts connections from the local machine
only. The first time, macOS may ask whether to allow incoming connections.

`cargo sim` is an alias for `cargo run -p smokebomb-simulator --features
simulator`. Set `PORT` to use a port other than 3000.

## Firmware

```sh
cargo test -p smokebomb-core                 # state machine, motion, rolls, rendering, end-to-end throw
npx nx run firmware:test                     # core + HAL crates
npx nx run firmware:lint                     # clippy for host, simulator board and nRF board
cargo fw                                     # no_std build for the nRF54L15 (thumbv8m.main-none-eabihf)
```

`cargo fw` builds `smokebomb-firmware --features nrf54l15` against the stub
HAL. Every driver still returns `NotImplemented`. See
`packages/firmware/zephyr/README.md` for the planned Zephyr integration.

To add a peripheral:

1. Add the trait to `packages/firmware/hal/src/lib.rs` and an associated type to `Platform`.
2. Implement it in `hal/simulator` (with state in `SimState` if the UI should see it) and in `hal/nrf54l15`.
3. Use it from `core` through `Peripherals<P>`.

## Server

```sh
npx nx run server:db-up        # postgres:16 on :5432 (user/pass/db: smokebomb)
cargo run -p smokebomb-server  # → http://127.0.0.1:8080, applies migrations on start
```

Environment variables (defaults in brackets):

| Var | Default |
|---|---|
| `DATABASE_URL` | `postgres://smokebomb:smokebomb@localhost:5432/smokebomb` |
| `BIND_ADDR` | `127.0.0.1:8080` |
| `JWT_SECRET` | `dev-only-insecure-secret` |
| `RUN_MIGRATIONS` | `1` |

Tests:

```sh
cargo test -p smokebomb-server                                    # DB tests are skipped
DATABASE_URL=postgres://smokebomb:smokebomb@localhost:5432/smokebomb \
  cargo test -p smokebomb-server                                  # includes migrations + roll verification
```

### Verifying a simulator roll against the local server

With both the simulator and the server running:

```sh
# 1. Register the simulated die
curl -s localhost:3000/api/device | \
  jq '{serial, public_key, firmware_version}' | \
  curl -s -XPOST localhost:8080/v1/devices -H 'content-type: application/json' -d @-

# 2. Throw the die in the browser, then send the last roll to the server
curl -s localhost:3000/api/state | jq '.last_roll' | \
  curl -s -XPOST localhost:8080/v1/rolls/verify -H 'content-type: application/json' -d @-
# → {"valid":true,"digest":"…","chain":"genesis","reason":null}
```

The simulator's roll payload uses the same field names as the verify request,
so you can pipe it straight through. Change any value and verification fails.

## Mobile

```sh
cd packages/mobile
./gradlew :shared:testDebugUnitTest          # shared model tests
./gradlew :composeApp:assembleDebug          # Android APK
./gradlew :composeApp:linkDebugFrameworkIosSimulatorArm64
```

iOS app:

```sh
cd packages/mobile/iosApp
xcodegen generate            # creates iosApp.xcodeproj (git-ignored)
open iosApp.xcodeproj        # run on an iOS simulator
```

The Xcode build runs `./gradlew :composeApp:embedAndSignAppleFrameworkForXcode`
before compiling Swift, so there is no separate Kotlin step. The Nx targets
`mobile:build-android`, `mobile:build-ios` and `mobile:test` wrap the same
commands. You can also open `packages/mobile` directly in Android Studio.

## Nx cheatsheet

```sh
npx nx show projects                 # shared, firmware, simulator-server, simulator-web-ui, server, mobile
npx nx run-many -t lint test         # every project that has these targets
npx nx affected -t test              # only what changed since main
npx nx graph                         # dependency graph in the browser
```
