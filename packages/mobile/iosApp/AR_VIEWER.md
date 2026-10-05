# AR viewer

The **Simulator** tab, iOS only (it began as the AR tab). You place a
Sugarcube at true size, on your real table through the camera or on a
virtual one (see [AR on or off](#ar-on-or-off)), then you can:

- switch between the models
- turn and move the die, and scale it if you unlock true size
- look inside with x-ray and explode it
- tap a part to see its name
- throw the die and read which face lands up

It's written in Swift (RealityKit and ARKit) and embedded in the Compose app.
It needs iOS 18 or later, and runs on iPhone and iPad.

- **iPhone, or any compact width:** the controls float over the camera view.
- **iPad:** the controls sit in a side panel next to the camera view. The panel
  includes a control pad: drag on it to turn the die, flick it to throw the
  die, while you watch the table.

## AR on or off

The tab is labelled **Simulator**. A small **AR** pane on the left edge of
the view switches where the die is. AR is **off** by default.

- **Off:** the die is on a virtual table in a dark studio, seen from a fixed
  camera close up (32° field of view, looking from the front right, as the
  desktop simulator frames it). It's placed straight away and locked in place:
  - Dragging turns the die rather than moving it.
  - A flick on the die, or **Roll**, winds up low (1.5 cm,
    `lockedWindupLift`), then tosses the die straight up (`lockedTossLift`,
    about 2 cm at the top), so it stays in frame. It's still a real throw to
    the firmware: picked up, shaken, airborne for a few ticks, a hard landing
    (`cargo test -p smokebomb-ffi` checks this). It tumbles inside a tight ring
    of walls (`lockedCorralFactor`). Once it's at rest, it slides back to its spot,
    keeping the face it landed on. The slide is gentle enough
    (`lockedReturnAcceleration`, about 0.04 g) that the firmware still reads
    the die as resting, so the reveal isn't cut short.
  - The camera frames the die to fit the view's shape (`studioDieFraming`).
    When the menu opens, the die lifts a little (`studioHeldHeight`) and the
    camera glides back to take in the die and its turn pad. It glides in
    again when the menu closes.
  - Everything else works the same: live firmware screens, touch, the menu
    and its turn pad, x-ray, explode and tap-to-name.
  - It needs no camera, so it also runs without camera access, on devices
    without AR, and in the iOS Simulator.
- **On:** the die is on your real table through the camera, and moves and
  rolls freely. This needs camera access and an AR-capable device. If camera
  access is off, the tab offers **Use without AR**.

Switching rebuilds the view, so the die is placed again and the firmware
reboots. The choice is remembered. The pane is hidden on devices without AR.
The studio's framing is `studioFieldOfView` and `studioCameraDistance` in
`DiePhysics.swift`; the studio itself is `buildStudio()` in
`ARSceneController.swift`.

## Where things are

| Path | What |
|---|---|
| `iosApp/ARViewerFactory.swift` | Makes the tab's view controller for Compose |
| `iosApp/ARViewer/ARViewerScreen.swift` | SwiftUI: iPhone overlay, iPad side panel, control pad, the camera-denied and unsupported-device states |
| `iosApp/ARViewer/ARViewerModel.swift` | State shared by both layouts (`@Observable`) |
| `iosApp/ARViewer/ARSceneController.swift` | `ARView`, session, coaching, placing, gestures, physics |
| `iosApp/ARViewer/DieRig.swift` | Finds contract prims in a loaded model: explode, shell fade, tap picking, highlight |
| `iosApp/ARViewer/ModelCatalog.swift` | The model list and loading behind `ModelSource` |
| `iosApp/ARViewer/PartLabels.json` | Tap labels; edit freely |
| `iosApp/ARViewer/DiePhysics.swift` | Tunable physics constants, the throw windup, face-up and settle maths |
| `iosApp/ARViewer/DieFirmware.swift` | What the viewer needs from the firmware, the panels per die, screen axes |
| `iosApp/ARViewer/ImuSynth.swift` | Die motion → IMU readings at exact 60 Hz ticks |
| `iosApp/ARViewer/LiveScreens.swift` | The firmware's panels on the die's faces |
| `iosApp/Firmware/RustDieFirmware.swift` | The firmware over the Rust C ABI (app target only) |
| `scripts/build_firmware.sh` | Builds `packages/firmware/ffi` for the SDK Xcode is building for |
| `iosApp/Resources/Models/` | Bundled `.usdz` files (a folder reference; Git LFS) |
| `iosAppTests/` | `ARViewerTests`: true-size check, explode, picking, labels, face-up |
| `scripts/convert_usdz.py` | Binary-root conversion and ARKit checks |
| `scripts/make_contract_fixture.py` | Generates the per-part stand-in model |

In Kotlin, `composeApp/src/commonMain/.../ar/ArViewer.kt` is the interface the
tab shows. `iosMain/.../ar/UIKitArViewer.kt` embeds the Swift view controller
with `UIKitViewController`. The tab only appears on platforms that supply an
`ArViewer`, so it is hidden on Android.

## Converting models

The hand-written models have a text (`.usda`) root layer, but ARKit expects a
binary `.usdc` root. Convert them once before bundling:

```sh
cd packages/mobile/iosApp/scripts
python3 -m pip install -r requirements.txt     # usd-core
python3 convert_usdz.py ~/Downloads/sugarcube_*.usdz
```

### The lid's etching

Each die's export carries the laser etching the desktop simulator draws on the
charging face (`drawEtching` in `packages/simulator/web-ui/src/shell.ts`):

- the **Sugarcube** wordmark and *Designed in Williamsburg, BK · Shake well
  before serving*, in Pacifico
- `SC-1 · S/N 000042` (the simulator's serial before a roll carries one)
- CE, the crossed-out bin and *Regulatory info in settings*, in Space Grotesk

The fonts are the firmware's (`packages/firmware/assets/fonts`).

The lid's flat faces round the window are a mesh of their own, `Etching`,
next to the die's `Shell`, on the same points and normals. It has UVs across
the flat face and a copy of the shell's material with the etching in its
colour, roughness/metallic and normal textures. Where it's marked: 55 % toward
the simulator's grey ink (`rgb(150,152,156)`), roughness 0.7 and metallic
0.3, with the edges as a slight recess. Everything else is the shell's own
look, including a tinted titanium's colour and an x-ray's opacity.

| Die | Flat face | Window | Script lines |
|---|---|---|---|
| 30 mm | 25 mm | 17.5 mm | centred in the band |
| 34 mm | 29 mm | 24 mm | the lowest tail 0.8 mm off the window |
| 40 mm | 35 mm | 26 mm | centred in the band |

The bin sits in the gap after "CE".

For each file, the script does what `usdcat in.usdz -o tmp.usdc` and then
`usdzip` would do:

1. It flattens the stage into one binary layer.
2. It repackages that layer with its textures into `iosApp/Resources/Models/`.

It makes one metadata-only fix along the way. Prims that bind a material but
don't declare `MaterialBindingAPI` get the schema applied. Current USD and ARKit
require it, and the report says how many prims it touched.

It then checks the result. It never changes geometry; it reports problems.

- **Package layout:** a `.usdc` root, only ARKit file types, stored entries
  aligned to 64 bytes.
- **Stage metadata:** a `defaultPrim`, `upAxis` Y and `metersPerUnit` 1.
- **Prim types and shader ids** that ARKit accepts.
- **Textures:** every texture path resolves to a file inside the package.
- **USD's validators:** every `UsdValidation` validator in this USD build.
- **True size:** the bounds match the table below to ±0.1 mm, with the origin
  at the bottom centre.
- **`usdchecker --arkit`:** run as well when that tool is on your PATH.
  `pip install usd-core` doesn't include the command-line tools.

At the end it prints how many per-part contract prims each file has, and how
many of its die shells have an `Etching`. It warns about any that don't.

- To check files that are already bundled without rewriting them, add `--check-only`.
- The exit status is 1 if any file has errors.

## Adding a model

1. Export it real-size, with 1 unit = 1 m, Y up, the origin at the bottom
   centre and the lid face down. Name it `sugarcube_<something>.usdz`.
2. Add its size to `EXPECTED_SIZE` in `scripts/convert_usdz.py`. Then run the
   script: it puts the converted file in `Resources/Models/`.
3. Add a `SugarcubeModel` to `ModelCatalog.all` with these fields:
   - id (the file name without `.usdz`), size, variant and kind
   - bounds in metres
   - mass, or nil if it can't be thrown
   - and, where they apply: `xray` (a die's x-ray), `die` (an x-ray's die),
     `exploded` (the pre-exploded counterpart)
4. Add a one-line test to `iosAppTests/ModelScaleTests.swift`.
   `testEveryCatalogModelHasATest` fails until you do.

`Models` is a folder reference, so you don't need to run `xcodegen generate`
just to add a file. The picker lists only models that are actually bundled.

The converted models are in Git LFS (`.gitattributes`), so install it once
(`brew install git-lfs && git lfs install`) before you clone or pull. Without
it you get small pointer files instead of models, and the picker shows none of
them. The contract fixture stays in plain git. Commit only the converted files,
and re-export rarely: every version stays in LFS storage.

To load models from a server later, implement `ModelSource`, which has
`isAvailable` and `url(for:)`, and pass it to `ModelCatalog`. Nothing else
needs to change.

### Sizes

| File | Size (m) | Mass |
|---|---|---|
| `sugarcube_30`, `_30_rainbow`, `_30_xray` | 0.030 cube | 0.055 kg |
| `sugarcube_34`, `_34_1bit`, `_34_xray` | 0.034 cube | 0.082 kg |
| `sugarcube_40`, `_40_xray` | 0.040 cube | 0.129 kg |
| `sugarcube_30_xray_exploded` | 0.0687 cube | (not thrown) |
| `sugarcube_lineup` | 0.156 × 0.040 × 0.040 | (not thrown) |
| `sugarcube_contract_fixture` | 0.030 cube | 0.055 kg (debug builds only) |

## Prim-naming contract

Today's models group geometry by material. These names work today:

- `Shell`, `SapphireWindows`, `LidScrews`, `ScrewSleevesAndSlots`, `LidSeam`
- `Screen_px` … `Screen_nz`
- in the x-rays, `Internal_<part>` and the wiring `Internal_w_*`

Tapping any of these names it.

`Etching` (the lid's etching, above) sits next to each `Shell`. It has no label,
so a tap on the lid still names the shell, and the screws flush with it still
win their taps.

Per-part models put every physical part in its own prim under the default prim:

| Prim | What |
|---|---|
| `Shell` | The titanium body |
| `Lid` | The bottom (`ny`) face |
| `Window_<face>` | Sapphire window, for each face in `px nx py ny pz nz` |
| `Module_<face>` | Panel glass, driver chip, ribbon and that face's `Screen_<face>` |
| `Screw_0` … `Screw_3` | Lid screws |
| `Pillar_0` … `Pillar_3` | Screw pillars |
| `Board`, `Cell`, `BalancePlate`, `Wiring` | Internals |

What each feature needs. When the prims are missing, the feature turns off
rather than breaking:

- **Explode slider:** any `Window_*` or `Module_*`. Each `Window_<face>` moves
  out along its face normal by `0.026 × e` m and each `Module_<face>` by
  `0.014 × e` m, for `e` from 0 to 1 (`DieRig.windowTravel`, `moduleTravel`).
  Everything else stays still.
- **X-ray:**
  - A model with `Shell` plus window or module prims: x-ray fades `Shell` to 25%
    opacity (`DieRig.xrayShellOpacity`).
  - Otherwise: x-ray swaps to the matching `_xray` file. The 30 mm x-ray also
    offers the pre-exploded file.
- **Tap to name:**
  - The label comes from the outermost labelled prim around the mesh you hit.
    For example, a tap on `Module_py`'s screen names the module.
  - In x-ray, taps pass through the shell and windows to what's behind them.
  - When parts sit flush (a screw in the lid), the smaller part wins.

`sugarcube_contract_fixture.usdz` is crude boxes that follow this contract.
It's generated by `scripts/make_contract_fixture.py` and appears in debug
builds as "30 mm · Per-part test", so you can try explode, the shell fade and
tapping before the real per-part models exist.

## Labels

`iosApp/ARViewer/PartLabels.json` maps prim names to labels.

- A key ending in `*` matches by prefix, and the longest match wins.
- An exact name beats any prefix.
- Keys starting with `_` are ignored.

## Physics

The die is a box at its real size with the mass from the table, and starts
kinematic. A flick, the control pad or **Roll** makes it dynamic and throws it.

- **Table:** the collider is a static 6 m × 6 m slab whose top is the plane you
  placed the die on.
- **LiDAR devices:** the scanned room mesh collides as well (the scene
  understanding `.physics` option).
- **Settling:** the die counts as settled when it has been still for 0.35 s.
  The viewer then reports the face whose normal is closest to world +Y, and
  makes the body kinematic again.
- **Reset:** returns the die to where it was placed, or where you last moved or
  turned it.

Tunable constants, all in `DiePhysics.swift`:

| Constant | Default | Effect |
|---|---|---|
| `staticFriction` / `dynamicFriction` | 0.45 / 0.35 | Die surface |
| `restitution` | 0.30 | Bounce; lower for felt |
| `tableStaticFriction` / `tableDynamicFriction` / `tableRestitution` | 0.55 / 0.45 / 0.30 | Table surface (RealityKit combines the two) |
| `linearDamping` | 0.05 | Air drag |
| `angularDamping` | 0.6 | Stands in for rolling resistance |
| `throwSpeed` | 0.15–0.6 m/s | Along the table, scaled by flick speed |
| `throwLift` | 0.35 m/s | Upward part of every throw |
| `corralRadius` / `corralHeight` | 0.18 m / 0.15 m | Invisible walls around where a throw starts; nil for none |
| `handlingMaxLinearMg` | 400 mg | Cap on linear acceleration while the die is slid or turned, so handling never reads as a shake |
| `dragFollowRate` | 18 /s | How fast a dragged die catches up with the finger |
| `heldHeight` / `heldMaxTilt` | 0.08 m / 0.6 rad | Held for the menu: height, and most tilt toward the camera |
| `heldMoveTime` / `tipTime` | 0.5 s / 0.35 s | Lifting or setting down; one quarter turn |
| `throwSpin` | 10–30 rad/s | Tumble |
| `flickThreshold` / `flickForFullSpeed` | 900 / 3000 pt/s | What counts as a flick; what's a hard one |
| `windupLift` / `windupLiftTime` | 0.05 m / 0.3 s | Picked up before a throw (eased, under ~0.35 g) |
| `windupShakeTime` | 0.5 s | Shaken in the air before release |
| `shakeAmplitude` / `shakeFrequency` | 7.5, 2, 5.7 mm / 6.5, 7, 7.5 Hz | About 1.3 g along the table, 0.4 g up |
| `shakeWobble` | 0.15 rad | Rocking while shaken |
| `continuousCollisionDetection` | on | Stops a fast, small die passing through the table |
| `useSceneReconstruction` | on | Room mesh as a collider on LiDAR devices |
| `settleLinearSpeed` / `settleAngularSpeed` / `settleTime` | 4 mm/s / 0.15 rad/s / 0.35 s | When it counts as stopped |
| `maxRollTime` | 8 s | Read the face anyway after this |

These are starting points and haven't been tuned on a device. PhysX is tuned
for objects about a metre across. If a 30 mm die floats or jitters, try more
`angularDamping` and less `restitution` first.

## Live screens: the firmware in the loop

With **Live screens** on (the default), the real firmware core runs on the
phone and drives the die's six screens. You see what the hardware would show:
the boot, the smoke filling as you shake, the roll and its reveal, the menu.

```
RealityKit pose ──► ImuSynth ──► sb_die_tick(imu, touch) ──► six panels ──► LiveScreens
   (60 Hz ticks)    accel+gyro      packages/firmware/ffi       RGBA          quads on the die
```

- **The firmware.** `packages/firmware/ffi` is a Rust static library with a C
  ABI (`include/smokebomb_ffi.h`). It boots `smokebomb_core::Firmware` on the
  simulator HAL with the clock driven tick by tick: the same core the board
  and the desktop simulator run, unmodified.
  - The 34 mm dice get the 96×96 grey build.
  - The 30 mm dice get the 64×64 RGB565 build.
  - The 40 mm die borrows the 30 mm colour build until it has a target of its own.
  - The line-up has no live screens.
- **Building it.** Xcode runs `scripts/build_firmware.sh` before each build.
  It builds the library for the device or the Simulator, so you need Rust
  (https://rustup.rs); the iOS targets are added on first use. Cargo only
  rebuilds when the firmware changes.
- **Motion in** (`ImuSynth.swift`):
  - Every 1/60 s, the die's pose at that instant is interpolated between
    rendered frames (`TickClock`).
  - It's turned into what the IMU would read. The accelerometer reads 1 g up
    plus the linear acceleration (from the last three positions); the gyro
    reads the turn since the last tick. Both are in the die's frame, as the
    desktop simulator's `World::imu` does it.
- **Throws** start with a windup: the die is lifted 5 cm and shaken for half a
  second, then let go. The firmware only counts a throw that starts in the
  hand (Held, then Shaking, then FreeFall). The windup is tuned to read
  between 0.6 g and 2.2 g, so it never looks like a fall or an impact.
  `FirmwareLoopTests` checks that.
- **Touch in.** A finger on the die touches the face under it until it lifts.
  A drag, pinch or twist ends the touch.
- **Frames out** (`LiveScreens.swift`):
  - Each face's panel is drawn on a quad over that face's `Screen_<face>`.
  - The quad is turned to the firmware's own screen axes
    (`orientation::BASES`), so frames land as on the hardware, whatever the
    model's UVs.
  - Pixels are sampled nearest-neighbour and unlit.
  - The baked screens are hidden while live screens are on.
  - The quads ride along with explode.
  - If a frame shows upside down on device, flip `LiveScreens.flipVertically`.
- **Haptics.** The firmware's haptic effects play on the phone.
- **Debug builds** show the firmware's mode under the caption.

- **The menu.** Hold a finger on a face to open it. The die lifts off the
  table with the menu's face toward you, tilted up toward the camera, as if
  in your hand.
  - The **turn pad** is a pane of glass beside the held die, only while the
    menu is open. It lies in the plane of the die's menu screen, beside it on
    the screen's right, as if that screen carried on past the die's edge, and
    then stays put in the room like a real object.
    Tap its keys: ▲ ▼ are ∓90° about your right, ◀ ▶ are ∓90° about
    vertical, and the twist keys turn the die about the line of sight. The
    firmware reads the turns from the gyro, as on the hardware
    (`MenuPanel.swift`).
  - When the menu closes, the die is set back down, flat on its lowest face.
- **Handling.** While the die is slid or turned on the table, its linear
  acceleration is capped at 0.4 g, so a finger's jitter can't read as a shake.
  A throw's windup and flight read everything.

Switching between models with the same panels keeps the firmware running.
Switching panels boots it again, and so does placing the die.

## Gestures

| Gesture | Does |
|---|---|
| Tap the table | Place the die (once a plane is found) |
| Tap the die | Live screens: touch the face (a hold is a long press). Otherwise, or in x-ray: name the part; tap elsewhere to clear |
| Drag on the die | Move it along the table |
| Flick on the die | Throw it: picked up, shaken, let go |
| Drag elsewhere, or twist | Turn it |
| Pinch | Scale, only when **True size** is unlocked; tap the % to return to 100% |
| Turn pad ▲ ▼ ◀ ▶ ⟲ ⟳ (menu open) | The menu's tips: quarter turns about your right, vertical and line of sight |

## Tests

The `ARViewerTests` scheme compiles the `ARViewer` sources and the bundled
models into an unhosted test bundle, so it needs neither Kotlin nor a camera.
CI runs it on a simulator.

- `ModelScaleTests`: every bundled model's visual bounds match the table to
  ±0.1 mm, with the origin at the bottom centre. A model that isn't bundled
  yet is skipped.
- `DieRigTests`: explode distances, the shell fade and tap picking, on the
  contract fixture.
- `ViewerLogicTests`: face-up, settling, ray picking, labels and captions.
- `FirmwareLoopTests`: IMU readings at rest, tilted, falling and spinning; the
  windup's limits; 60 Hz ticks at any frame rate; touch faces; screen axes.

The firmware side has its own tests (`cargo test -p smokebomb-ffi`). They boot
both builds and play throws through the C ABI, including the viewer's windup
and flight rebuilt from positions, and check they end in a reveal.

Debug builds also show the measured bounds under the caption, green when they
are within ±0.1 mm.
