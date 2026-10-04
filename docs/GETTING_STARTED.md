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
npm install
npm run build -w @smokebomb/simulator-web-ui   # after every pull or branch switch
cargo run --features=simulator                 # → http://localhost:3000
```

Or run both with Nx, which rebuilds the UI first: `npx nx run simulator-server:serve`.

The built UI (`web-ui/dist`) isn't in git, so `cargo run` serves whatever you
built last. If it's older than the server, the browser shows a banner saying
the page is out of date, and the server logs a warning at startup and when an
old page connects. Rebuild the UI and hard-reload the page.

The die boots with the mockup's pip animation, then shows its setup. Throw it
to see a result; hold a screen to open the menu.

In the browser:

- **Hold to shake, release to throw** works like the mockup's throw button.
  Holding shakes the die in the hand; releasing throws it. The die tumbles and
  lands, the firmware rolls and signs, and the result shows on every face
  except the one facing down. The roll appears in the **Signed rolls** list
  with its digest and signature.
- **Tip to an adjacent screen:** the pad does a quick quarter-turn that brings
  a neighbouring screen to the front. The arrow keys do the same, and while
  the menu is open so does a swipe on the die. In the menu, ▲/▼ change the
  value and ◀/▶ change the page (SIM_SPEC C3). The firmware reads tips from
  the gyro, the way the real die will.
- **Turn several screens at once:** in the menu, hold F while you drag, or
  tick **Multi-turn swipes in the menu** (for touch screens). The die
  follows the drag about one axis, across as many screens as you like, and
  settles on the nearest one when you let go. Each screen it
  passes is one step: two screens left is two pages on, three screens up is
  three values on.
- **Drag** on the die to turn it in your hand.
- **Face up** sets the die down with that screen on top.
- **Press and hold a screen** for the menu: the ring fills, the menu opens
  and the die turns that screen toward you. Tap on a Settings item to change it. Hold again to save and
  return to the roll; throwing or leaving it for 25 s discards the changes.
- **On charging nest** docks the die; it settles upright.
- **Reduced motion** shortens the tumble and tips, as in the mockup.

The server owns the die's position and orientation and generates the IMU
readings the firmware sees from them; the browser only draws. See
[ARCHITECTURE.md](ARCHITECTURE.md#simulator).

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
cargo test -p smokebomb-core                 # state machine, motion, rolls, menu, rendering, screen snapshots
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

## Screen snapshots

```sh
cargo test -p smokebomb-core --test snapshots                      # compare
UPDATE_SNAPSHOTS=1 cargo test -p smokebomb-core --test snapshots   # regenerate
```

Every face at chosen moments of a scripted run (boot, tap, throw, menu, ...)
is compared with a PNG sheet in `packages/firmware/core/tests/snapshots/`.
After a change to the screens you meant to make, regenerate the sheets, look
at them, and commit them with the change. A failing run prints each face's
difference and writes expected, actual and difference images to
`target/snapshot-diffs/`; CI uploads those as the `snapshot-diffs` artifact.

To rebuild the asset pack (fonts + placeholder clips) as a file for flashing:
`cargo run -p smokebomb-assets-build -- smokebomb.smkb`.

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

The **AR** tab needs a real iPhone or iPad; the Simulator has no AR. See
[AR_VIEWER.md](../packages/mobile/iosApp/AR_VIEWER.md) for converting and
bundling the models and for its tests (the `ARViewerTests` scheme).

### The app with the simulator as its die

The app can use the desktop simulator instead of a real die: it speaks the
die's BLE messages to the simulator over a WebSocket (`/phone`, see
[STORE.md](STORE.md#simulator-link)). On the **Die** tab, under *No die? Use
the simulator*, enter the simulator's address and tap **Connect to
simulator**. Then you can set the dice, turn modes on and off, and see each
throw's signed roll arrive. The **History** tab lists every roll the
simulator kept since it started (it syncs on connect), then adds new ones as
they land. Keep the simulator's browser page open to throw
the die and to watch the menu change.

| App runs on | Start the simulator with | Address in the app |
|---|---|---|
| iOS simulator, same Mac | `cargo run --features=simulator` | `localhost:3000` |
| iPhone on the same Wi-Fi | `HOST=0.0.0.0 cargo run --features=simulator` | `<your-mac>.local:3000` or the Mac's IP |
| Android emulator | `cargo run --features=simulator` | `10.0.2.2:3000` |

To put the app on your own iPhone: connect the phone by cable (or pair it for
wireless debugging in Xcode's *Devices and Simulators*), turn on Developer
Mode on the phone (*Settings → Privacy & Security*), then set your team
once in `packages/mobile/iosApp/Local.xcconfig` (git-ignored, so
`xcodegen generate` keeps it):

```sh
cd packages/mobile/iosApp
cp Local.xcconfig.example Local.xcconfig   # set DEVELOPMENT_TEAM
xcodegen generate
```

A free Apple ID works; its builds expire after seven days. Find your team ID
in Xcode under *Settings → Accounts*. If the bundle id `com.smokebomb.app` is
taken, set `SUGARCUBE_BUNDLE_ID` there too. Pick the phone as the run destination and
press Run. The first time, trust the developer profile on the phone
(*Settings → General → VPN & Device Management*). The app asks for local
network access when it first connects to the simulator; allow it.

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
