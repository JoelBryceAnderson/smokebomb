# Store: modes and themes

The phone app decides what a Sugarcube offers: which game modes appear on its
Mode page, and which theme it draws with. Both are sold in the in-app store.
This document is the design; the [status table](#status) says what exists.

## Two kinds of item

| | Mode | Theme |
|---|---|---|
| What it is | Code: a `PlayMode` with its own menu page, rules and screens | Data: an SMKB asset pack (sprites, parameters, fonts) |
| How it reaches the die | Already there. Every mode ships in the firmware; new modes arrive with a firmware update (DFU) | Downloaded by the phone and streamed to QSPI flash over BLE |
| What a purchase gives | A license that unlocks it on a die | A license plus the pack |
| Example | Pig Toss | Ember |

Modes can't be downloaded like themes: they're Rust in the firmware image,
and a scripting VM on a 256 KB MCU is out of scope. So modes follow **ship
everything, unlock by license**. A mode that needs art (Pig Toss's pigs) can
name a required asset pack, which the phone installs with the license.

## Licenses

The die enforces what it plays, not the app: a modified app must not be
able to switch on a mode nobody paid for.

- The server keeps purchases per **account** (`theme_purchases` today,
  generalised to store items).
- When the owner installs an item on a die, the server issues a
  **license** for that die (`smokebomb_shared::License`):

  ```text
  message = "SCLIC\0\0\x01" | device serial (9) | kind u8 | item id | issued_at u64 BE
            kind 0 = mode: id is the ModeId byte
            kind 1 = theme: id is the pack's SHA-256
  signature = P-256 ECDSA over SHA-256(message), raw r || s
  ```

- The store's public key is compiled into the firmware. The die accepts a
  license only if the signature checks out and the serial is its own, then
  keeps it in internal flash. Licenses are the die's, not settings: loading
  or replacing settings never changes them (`Firmware::set_settings`).
- An account can install an item on a limited number of dice (a server
  setting, say 3). Selling a die with its licenses works because they're
  bound to the die, not the account.
- Dice mode is always licensed. The four current modes ship free, so a
  fresh die starts with all of them licensed; modes added later start
  unlicensed.

## Enabled modes

`Settings` keeps two sets (`ModeSet`, one bit per `ModeId`):

- `licensed`: what the die may play.
- `enabled`: what the owner turned on in the app.

The Mode page offers `licensed ∩ enabled`, always with Dice. With only Dice
left there is no Mode page and the menu is the plain Count, Die, Settings
ring. If the mode in use is turned off or loses its license, the die plays
Dice, and the saved mode comes back if it's turned on again.

Mode page order is wire order for now. Reordering from the app needs an
ordered list in `Settings` instead of a bitset: a later step.

## BLE messages

In `smokebomb_shared::protocol`, mirrored in Kotlin:

| Direction | Message | Purpose |
|---|---|---|
| phone → die | `GetInventory` | Ask what the die can play |
| phone → die | `SetEnabledModes(ModeSet)` | Choose the Mode page's modes |
| phone → die | `InstallLicense(License)` | Unlock a store item |
| die → phone | `Inventory { licensed, enabled, active }` | The answer, and pushed on every change (including a mode picked on the die) |
| phone → die | `BeginTheme`, `ThemeChunk`, `CommitTheme`, `SetTheme` | Theme upload and selection (planned) |

Theme upload: the phone sends the pack's length and SHA-256, then chunks
sized to the ATT MTU into a staging slot in QSPI. The die hashes as it
writes and only switches to the pack on a matching `CommitTheme`, so a
dropped connection never leaves a half-written theme in use.

## Server

- Generalise `themes` into store items: `kind ∈ {mode, theme}`, plus
  `mode_id` for modes, and `min_firmware` for both (a mode needs the firmware
  that contains it).
- `POST /v1/store/{slug}/purchase`: verify the App Store / Play receipt,
  record the purchase.
- `POST /v1/devices/{serial}/licenses` `{ item }`: owner only; checks the
  purchase and the per-account die limit, returns a signed `License`.
- The store signing key lives in the server's secrets (an HSM or KMS later),
  never in the repo.

## App

- **Die tab:** when connected, a Modes section lists every mode: toggles for
  licensed ones, a lock and a link to the store for the rest. Changes go out
  as `SetEnabledModes` and the list follows the die's `Inventory`.
- **Store tab:** modes and themes together. Buying uses in-app purchase
  (StoreKit on iOS, Play Billing on Android). The platform takes its cut,
  which is accepted. After purchase the app asks the server for a license
  and installs it. If the die's firmware is older than the item's
  `min_firmware`, the app offers the update first.

## Simulator link

To try all this without hardware, the simulator server accepts the same
`PhoneToDie` / `DieToPhone` messages on its `/phone` WebSocket (JSON, one
message per frame, serde's default shape) and feeds them to the firmware
between ticks, as the die drains BLE writes (`simulator/server/src/phone.rs`).
It greets a new phone with `Hello` and `Inventory`, answers every setup
change with `Inventory`, and pushes `Roll` and `Inventory` as they happen. It
trusts `InstallLicense` without checking the signature.

In the app, `SimulatorLink` is a `BleManager` that connects there by
`host:port`, and `DieLinks` switches the app between it and Bluetooth, so the
iOS simulator on the Mac, or an iPhone on the same Wi-Fi, can drive the
simulated die. See [Getting started](GETTING_STARTED.md#the-app-with-the-simulator-as-its-die).

## Status

| Piece | Status |
|---|---|
| `ModeId`, `ModeSet`, `License`, inventory messages (shared) | ✅ Defined |
| Firmware: licensed and enabled sets, Mode page follows them, `inventory` / `set_enabled_modes` / `unlock_mode` | ✅ Working, tested |
| Firmware: license signature check, store key, licenses in flash | 🗓 Planned |
| App: Modes and Dice sections on the Die tab | ✅ Working against the simulator; Bluetooth still stubbed |
| Simulator link over WebSocket (`/phone`, `SimulatorLink`) | ✅ Working: modes, dice setup, live rolls |
| Theme upload over BLE, theme selection | 🗓 Planned |
| Server: store items, purchase, license issuing | 🗓 Planned |
