# AR viewer

The **AR** tab, iOS only. You place a Sugarcube on your table at true size,
then you can:

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
| `iosApp/ARViewer/DiePhysics.swift` | Tunable physics constants, face-up and settle maths |
| `iosApp/Resources/Models/` | Bundled `.usdz` files (a folder reference) |
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

For each file, the script does what `usdcat in.usdz -o tmp.usdc` and then
`usdzip` would do:

1. It flattens the stage into one binary layer.
2. It repackages that layer with its textures into `iosApp/Resources/Models/`.

It then checks the result. It never changes geometry; it reports problems.

- **Package layout:** a `.usdc` root, only ARKit file types, stored entries
  aligned to 64 bytes.
- **Stage metadata:** a `defaultPrim`, `upAxis` Y and `metersPerUnit` 1.
- **Prim types and shader ids** that ARKit accepts.
- **USD's validators:** every `UsdValidation` validator in this USD build.
- **True size:** the bounds match the table below to ±0.1 mm, with the origin
  at the bottom centre.
- **`usdchecker --arkit`:** run as well when that tool is on your PATH.
  `pip install usd-core` doesn't include the command-line tools.

At the end it prints how many per-part contract prims each file has.

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
| `throwSpeed` | 0.25–1.4 m/s | Along the table, scaled by flick speed |
| `throwLift` | 0.5 m/s | Upward part of every throw |
| `throwSpin` | 10–30 rad/s | Tumble |
| `flickThreshold` / `flickForFullSpeed` | 900 / 3000 pt/s | What counts as a flick; what's a hard one |
| `releaseHeight` | 0.02 m | Lift before release |
| `continuousCollisionDetection` | on | Stops a fast, small die passing through the table |
| `useSceneReconstruction` | on | Room mesh as a collider on LiDAR devices |
| `settleLinearSpeed` / `settleAngularSpeed` / `settleTime` | 4 mm/s / 0.15 rad/s / 0.35 s | When it counts as stopped |
| `maxRollTime` | 8 s | Read the face anyway after this |

These are starting points and haven't been tuned on a device. PhysX is tuned
for objects about a metre across. If a 30 mm die floats or jitters, try more
`angularDamping` and less `restitution` first.

## Gestures

| Gesture | Does |
|---|---|
| Tap the table | Place the die (once a plane is found) |
| Tap the die | Name the part; tap elsewhere to clear |
| Drag on the die | Move it along the table |
| Flick on the die | Throw it |
| Drag elsewhere, or twist | Turn it |
| Pinch | Scale, only when **True size** is unlocked; tap the % to return to 100% |

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

Debug builds also show the measured bounds under the caption, green when they
are within ±0.1 mm.
