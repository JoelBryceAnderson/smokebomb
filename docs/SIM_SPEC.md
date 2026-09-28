# Smokebomb: UX, visual and hardware spec for the simulator

**Purpose.** Make a firmware-driven simulator behave and look 1:1 like the interactive mockup (https://claude.ai/artifact/8Z1hGvE7k3mf5sGfaWEJ7j) and the hardware we've designed.

**Source.** Every number here comes from the mockup's code. **If this document and the mockup disagree, the mockup wins.** Flag the difference so this document gets fixed.

**Status.** Checked line by line against the mockup source (artifact version `1790415968-5231`) on 2026-09-28. The differences found are folded into the text below and listed in [Part G](#part-g-corrections-log). Design decisions made on 2026-09-28 are in [Part H](#part-h-decisions); where a decision departs from the mockup, the text follows the decision and says so.

**Conventions**
- **Units:** mm for hardware.
- **Screen units** come in three forms:
  - **Panel px:** the real 96×96 pixel grid, origin at the top-left, center at (48, 48) in continuous coordinates.
  - **Mockup canvas units:** the mockup draws each face on a 256×256 canvas that spans **±1 scene unit (26.5mm)**. That is larger than the 24mm window and smaller than the 29mm flat face, so its outer part is never visible. The lit area is the centered square **±83 canvas units** (166 units = 96 px). The real panel's lit area is 17.26mm; the mockup draws it at 166/256 × 26.5 = **17.18mm**, a 0.5% difference that doesn't matter.
  - **Converting:** panel px = (canvas − 128) × **0.5783** + 48, and sizes × 0.5783.

  Both are given where it matters. The mockup renders at canvas resolution, then downsamples to 96×96, so firmware drawing natively at 96×96 should use the panel-px values.
- **Timing:** seconds.
- **"Ease-out cubic"** means 1 − (1 − x)³. **"Smoothstep"** means x²(3 − 2x).

---

## Part A: Hardware and physical appearance

### A1. Body
| Item | Value |
|---|---|
| Form | Cube with uniformly rounded edges and spherical corners (one radius everywhere) |
| Outer size | **34.0 mm** |
| Edge / corner radius | **2.5 mm** (matches the screen corner radius) |
| Flat face region | 29.0 mm square (34 − 2 × 2.5) |
| Default orientation at load | Rotated 0.35 rad about vertical |
| Weight (for physics) | ≈57 g titanium; ≈50 g aluminum; ≈64 g ceramic; ≈73 g steel. Centered mass. |

Mockup scale: 1 scene unit = 13.25 mm, so the die half-size is 1.2830 u, the edge radius 0.1887 u, and the face-window half-size 0.906 u.

### A2. Each face, from the outside in
| Layer | Size | Corner radius | Notes |
|---|---|---|---|
| Metal face (flat) | 29.0 mm square flat, rolling into the 2.5 mm edges | — | Metal border between the window and the cube edge: **2.5 mm flat + 2.5 mm roll = 5.0 mm** from window edge to cube edge |
| **Window (sapphire cover glass)** | **24.0 mm square** | **2.52 mm** | Sits ~0.15 mm below the metal surface (effectively flush). Glossy black glass. |
| Black ink border (inside the window) | **3.37 mm** wide on every side | — | Unlit black glass between the window edge and the lit area |
| **Lit area (OLED active area)** | **17.26 mm square** (the mockup draws 17.18 mm; see Conventions), centered | **2.52 mm** (mask radius = 14 panel px) | Rounded by the ink mask; pixels outside the rounded square never show |
| Pixel grid | **96 × 96**, pitch **0.18 mm**, dot **0.16 mm** (0.02 mm dark gap) | — | The mockup shows no gaps (3×3 texels per pixel). Gaps are optional, for close-up realism. |

Face indexing used throughout the mockup: **0 = +X, 1 = −X, 2 = +Y, 3 = −Y, 4 = +Z, 5 = −Z.**

### A3. Glass and screen material (mockup rendering)
- **Glass:** base color `#030304`, roughness 0.05, metalness 0, clearcoat 1.0, clearcoat roughness 0.03.
- **Screen:** an emissive map (the 96×96 output) at intensity **2.2 in day lighting, 4.8 in night mode**.
- **Screen texture:** 444×444 px spanning the full ±1 unit face square. The 96×96 image fills the centered **288×288** (3 px per panel pixel, nearest-neighbour, offset 78 px). The corners are masked with a rounded rect of radius **42 texture px**, and everything outside is black.

### A4. Finishes (shell material, PBR metal/rough)
| Key | Label | Base color | Metalness | Roughness | Extra |
|---|---|---|---|---|---|
| stealth (default) | Stealth black | `#232428` | 0.9 | 0.38 | — |
| chrome | Polished chrome | `#9AA0A8` | 1.0 | 0.04 | — |
| ceramic | White ceramic | `#F1EFEA` | 0.0 | 0.22 | — |
| glow | Glow ceramic | `#E6EEE6` | 0.0 | 0.30 | Emissive `#7DFFC4`, intensity 0.05 in day, **1.1 at night**. Selecting it switches to night mode. |
| gold | Gold | `#D6A847` | 1.0 | 0.16 | — |
| rainbow | Heat-tinted titanium | `#DCDCDF` | 1.0 | 0.22 | Heat-tint shader (below) |

**Heat-tint shader** (seamless, per pixel):
```
ndv  = clamp(dot(N, V), 0, 1)
t    = fract(ndv*0.9 + shift + dot(worldPos, vec3(0.05, 0.035, 0.045))) * 5
palette (loops): c0 (0.86,0.72,0.40) straw, c1 (0.74,0.46,0.24) bronze, c2 (0.48,0.32,0.66) violet,
                 c3 (0.28,0.44,0.82) steel blue, c4 (0.30,0.64,0.66) teal
heat = smoothstep blend of consecutive palette entries by t
albedo *= mix(vec3(0.78), heat, 0.8) * 1.3
shift  = 0.12 * (world X-axis of die).z + 0.10 * (world Z-axis of die).y   // slow drift as the die turns
```

### A5. Lighting and camera (to match the mockup's look)
- **Tone mapping:** ACES filmic, sRGB output. Exposure **0.8 in day, 0.32 at night**.
- **Environment:** a PMREM studio. Sky gradient from `#C9CED6` (top) through `#7A7F87` (40%) and `#2A2D32` (55%) to `#0C0D0F`. Emissive softboxes (w × h at position, color × power):

  | Size | Position | Color | Power | Role |
  |---|---|---|---|---|
  | 10×4 | (−8, 5, 6) | `#FFF2E2` | 3 | warm key |
  | 1.6×11 | (9, 2, −3) | `#E4EEFF` | 4 | cool vertical strip |
  | 8×8 | (0, 12, 0) | white | 1.4 | soft overhead |
  | 14×0.7 | (0, 1, 11) | white | 3 | thin front strip for edge glints |
  | 0.9×0.9 | (6, 7, 7) | white | 7 | small bright spot |
  | 0.9×0.9 | (−9, 3, −6) | white | 6 | small bright spot |
  | 0.7×0.7 | (3, −2, −10) | `#FFF0DC` | 5 | small bright spot |

- **Direct lights:**
  - directional key at (3, 6, 4): intensity 0.45 day, 0.05 night
  - ambient: 0.12 day, 0.02 night
- **Camera:** perspective, 32° vertical FOV, direction (0.55, 0.62, 1).
  - **Portrait** means viewport aspect (width ÷ height) < 0.75.
  - Distance ≈ 9.5 × 1.156 u in landscape and 12.5 × 1.156 u in portrait. (1.156 is K, the scale-up from an earlier 29.4mm body; the Nest and case dimensions are derived from it too.)
  - Look target Y offset −0.35 × 1.156 u (landscape) or −0.1 × 1.156 u (portrait).
  - Close-up: distance × 0.4, look at center.
  - Soft contact shadow under the die.
- **Idle turntable:** after 4 s without interaction, the die rotates about vertical at 0.18 rad/s. This is a mockup nicety only.
- **three.js version:** the mockup uses **r128**. Newer three.js (r152+) converts hex material colours from sRGB to linear, and (r155+) uses physical light units. Porting the numbers above to a current three.js without adjusting makes the scene visibly darker. Either disable `THREE.ColorManagement` and scale the directional and ambient intensities by π, or pin r128, and check the result against mockup screenshots.

### A6. Internals (not visible; for reference)
- **Panel module:** ER-OLED0.96-6W, SSD1317, 24.0 × 25.7 × 1.2 mm, 4-wire SPI, 16 gray levels, ≤100 Hz.
- **Other parts:** nRF54L15, nPM1300, ATECC608, LSM6DSx motion sensor, DRV2605L with an LRA, capacitive touch ring per face, battery ≈26×26×5 mm (≈250–290 mAh).
- **Sapphire:** ≈0.7 mm.
- **Cutaway:** https://claude.ai/artifact/T9NrkCTZnhfZx36cLZY7Le

### A7. Nest (charging dock), scaled to the 34 mm die
Units: mm, relative to the die center when docked (the die sits in the pocket).

| Item | Value |
|---|---|
| Footprint | **55.1 mm square**, plan corner radius **8.4 mm**, top and bottom edges softened by a 2.65 mm bevel |
| Height | 18.2 mm (bottom 29.9 mm below die center, top 11.6 mm below die center) |
| Pocket | **35.6 mm square**, corner radius 3.4 mm, **5.4 mm deep**. The die's bottom 5.4 mm sits in it. |
| Top inlay | Black pebbled leatherette, inset 3.4 mm from the outer edge, around the pocket |
| Band | Satin metal band matching the die's finish, ~1.5 mm tall, 6 mm above the bottom |
| Body | Black plastic `#070708`, roughness 0.5, low environment reflection (0.35) |
| Pocket floor | `#0D0E10` matte |

### A8. Case (the Duo; v2 accessory, still in the mockup)
| Item | Value |
|---|---|
| Plan | **91.9 × 49.6 mm**, corner radius 14.6 mm, 2.9 mm soft bevel |
| Body height | 27.6 mm (bottom 20.7 mm below die center, deck 6.9 mm above) |
| Lid thickness | 15.3 mm. The lid includes an 11.0 mm-deep rim with two recesses (36.1 mm square, radius 3.7 mm) that close over the dice. |
| Closed height | ≈42.9 mm |
| Hinge | Along the back edge. Shown open at 1.9 rad (≈109°). |
| Pockets | Two, 35.6 mm square, centers at ±21.1 mm. The dice stand ≈10 mm proud of the deck. |
| Materials | Pebbled black leatherette (dark grain, roughness 0.88); satin metal seam band 1.7 mm tall, inset 1.06 mm, 5.6 mm below the deck; deck and pocket `#0D0E10` |

---

## Part B: Screen rendering rules

### B1. Output pipeline (what the panel can show)
1. Draw each face's content. The mockup uses a 256×256 canvas; the lit area is the center ±83.
2. Downsample the lit area to **96×96**.
3. **Quantize to 16 levels (4-bit).** The mockup uses `level = min(15, floor(max(R,G,B)/255*15 + D[x,y]))` and displays `level*17`.
   - D is an ordered dither: `D = 0.02 + 0.96 * frac(0.7548776662*(x+1) + 0.5698402910*(y+1))`.
   - This is a light dither between neighbouring steps only, to hide banding.
4. The rounded corners (radius 14 panel px) are hidden by the **black ink mask on the cover glass**, not by the panel. The mockup applies it to the 3×-upscaled glass texture (radius 42 texture px), so it cuts through panel pixels at sub-pixel precision. The firmware does not need to mask; the simulator's glass renderer does. (The firmware may still skip drawing corner pixels to save power.)
5. Panels update at **≤100 Hz**. The mockup skips panel updates closer together than 9.5 ms.

Colors used on screen:
- Foreground white: `#F4F5F7`, which is effectively level 14–15.
- Dud gray: `#8A8C90`.

Glow: the mockup draws text with a soft glow (shadow blur of 18 canvas units for big numbers, 8 for small text, 10–14 for icons and menus). After downsampling this becomes a faint gray halo of about 1–3 panel px. Reproduce it as a 1–2 px low-level halo, or omit it.

**Font:** Space Grotesk. Weight 700 for numbers, values and titles; 600 for labels. Firmware needs bitmap cuts at the panel-px sizes below.

### B2. Text orientation
- **Side faces:** text is upright relative to gravity.
  1. Project world-up onto the face.
  2. Compute the raw angle.
  3. Snap it to 90° steps.
  4. Change the snapped angle only when the raw angle drifts more than **45° + 0.3 rad** from the current one. This hysteresis prevents flicker.
- **Top and bottom faces** (the axis of the face pointing up): keep a **locked orientation**, whatever they last had. The die can't know where the viewer is.
- **No clear up direction:** keep the current angle.

### B3. The face-down screen
The face touching the table is dark:
- no result,
- no smoke spawned on it (particles can still drift onto it and are drawn there),
- no wake label (decision H2; the mockup still draws it there and should be updated to match).

---

## Part C: States and behavior

Order of precedence each frame, per face:
1. Restart blackout.
2. Landing flash overlay.
3. Menu close fade.
4. Hold ring.
5. **Boot**, **docked**, **menu**, **result**, **success** or **wake label**, whichever applies, in that order.
6. Smoke and particles drawn on top, additively.

### C1. Boot (power-on or Restart only; never on wake)
- **Pips:** center-to-corner offset **38.2 canvas (22.1 px)**, pip radius **12.45 canvas (7.2 px)**.
- **Slot order** (each value keeps existing pips sliding and grows or shrinks the rest):

  | Value | Slots |
  |---|---|
  | 1 | c |
  | 2 | tl, br |
  | 3 | tl, br, c |
  | 4 | tl, br, tr, bl |
  | 5 | tl, br, tr, bl, c |
  | 6 | tl, br, tr, bl, ml, mr |

- **Start values** by face index 0–5: **3, 4, 1, 6, 2, 5** (opposites sum to 7). The top face starts at **6** so its loop lands on **5**.
- **Stagger:** each face starts 0.05 × face index s late (the top face has no stagger).

| Time (s from boot) | What happens |
|---|---|
| 0 → 0.25 | Pips appear (scale and alpha 0 → 1) |
| 0.25 → 1.95 | 5 steps, one every **0.34 s**; each tween takes **0.22 s** (smoothstep) and advances the value by +1 (wrapping 6 → 1) |
| side faces, after 1.95 | Fade over **0.4 s** while shrinking to 40% |
| top face, 1.95 + 0 → 0.35 | The four corner pips fly outward (distance × (1 + 1.4e²)), fade out and shrink by 40% |
| top face, +0.35 → 0.75 | The lone center pip "breathes" (scale 1 + 0.07·sin(4πu)) |
| top face, +0.75 → 0.90 | The center pip swells to 2.2× and fades. **Burst:** 170 smoke particles from the center (see D3) |
| top face, +1.65 → 2.25 | **"SMOKEBOMB"** fades in (700, 27 canvas / 15.6 px), holds, then fades from +3.8 to +4.2 |
| 6.2 s total | Boot ends and the **wake label** shows for 2.2 s |

The boot is interrupted by a throw or by opening the menu.

### C2. Wake and setup label
- **Tap** a screen: all faces show the setup label for **3 s**.
  - In the mockup, a tap is any press on the die that moves less than 8 px and is released before the 0.8 s hold completes. There is no shorter time limit. Hardware needs its own definition.
  - Taps do nothing while the menu is open.
  - Fades in over 0.35 s and out over 0.6 s, at 85% alpha.
  - Font 700, size `fitPx(label, 50)`: at most 50 canvas (28.9 px), shrinking so the label fits 150 canvas (87 px) wide.
- **Setup changes, the end of boot, and closing the menu** (saved *or* discarded): the wake label shows for **2.2 s**.
- **Label formats:**
  - `d20`
  - `3d6`
  - `Pass the Pot`
  - `Pass the Pot ×2`
  - short form in the menu status bar: `Pot ×2`

### C3. Menu (hold to enter, tip to navigate)
**Entering**
- **Press and hold** a screen, with the finger moving less than 8 px, for **0.8 s**.
- **Hold ring:** appears after **0.22 s** and fills over the remaining 0.58 s.
  - Shape: a rounded square inset **3 canvas** from the lit edge (square ±80 canvas, i.e. 1.7–94.3 px), corner radius **21 canvas (12.1 px)**, stroke **4 canvas (2.3 px)**.
  - It starts at **12 o'clock** and draws **clockwise**, as a dash of length progress × perimeter.
- **On open:**
  - haptic pattern [18 ms on, 40 off, 26 on],
  - the die visually pulses (scale +3.5%, sine over 0.28 s),
  - the die snaps (0.35 s, ease-out cubic) so the held face squarely faces the viewer. This snap is a mockup affordance; on hardware the person is already holding that face toward themselves.
  - Menu frame: front = the held face; right and up = the viewer's right and up.
  - All particles and any shown result are cleared.
- **Menu grows in** (0.35 s): the filled ring flashes outward (alpha 1 → 0, inset shrinking by 3 units per unit of progress), and the menu scales from 0.86 to 1 (ease-out cubic) while fading in over 0.3 s.

**Menu layout** (front face only; canvas units from the lit-area center → panel px)

| Element | Position (canvas) | Panel px | Style |
|---|---|---|---|
| Status: setup short label | left-aligned at (−66, −66) | x 9.8, y 9.8 | 700, 14 → 8.1 px, 85% |
| Status: battery | rect at (44, −71), 20×10, 1.5 stroke; nub 2×4 at (64, −68); fill inset 2, width 16 × level | x 73.4, y 6.9, 11.6×5.8 | 85% |
| Page title | centered at y −40 | y 24.9 | 600, 15 → 8.7 px, 75% |
| ▲ | y −22 | y 35.3 | 700, 13 → 7.5 px, 55% |
| Value | y +12 | y 54.9 | 700, `fitPx(val, 52)` → ≤30 px, full alpha |
| ▼ | y +46 | y 74.6 | 700, 13 → 7.5 px, 55% |
| Page dots (3) | y +66, spacing 14, radius 3.5 | y 86.2, spacing 8.1, r 2.0 | current 100%, others 35% |
| Settings page: item name | y +2 | y 49.2 | 700, `fitPx(name, 26)` → ≤15 px |
| Settings page: item value | y +27 | y 63.6 | 600, `fitPx(value, 18)` → ≤10.4 px, 72% |

`fitPx(str, max, width = 150)` = `min(max, floor(width / (len * 0.58)))` in canvas units (× 0.5783 for px).

**Pages** (tip left or right to change page, wrapping)

| # | Title | Values (tip up = next, down = previous; wraps) |
|---|---|---|
| 0 | How many dice | 1–10 (1–3 for Pass the Pot) |
| 1 | Which die | d4, d6, d8, d10, d12, d20, d100, Pass the Pot |
| 2 | Settings | Brightness 70%; Haptics Strong; Smoke Full; Large text Off; Sleep after 2 min; Night mode Auto; Bluetooth On; Verified rolls Off; Owner "Joel"; Restart "Hold to restart"; About "v0.1.0 · SB-0042". Values are display-only. |

Choosing Pass the Pot while the count is above 3 clamps the count to 3.

**Tips**

| Input | Die rotation | Effect |
|---|---|---|
| Tip up | −90° about the frame's right axis (the bottom face comes to the front) | next value |
| Tip down | +90° about the frame's right axis | previous value |
| Tip left | −90° about vertical (the right face comes to the front) | next page |
| Tip right | +90° about vertical | previous page |

Mockup input: a swipe of more than 36 px (by dominant direction), or the arrow keys.

**Tip animation**
- Duration **0.42 s**, ease-out cubic.
- The die hops up by 0.12 u (≈1.6 mm) on a sine arc.
- Haptic **10 ms**.
- Content during a tip:
  - Let **m** be the unit direction the face's surface moves during the tip, in that face's canvas coordinates, and D = 120 canvas (69 px).
  - Old face: the menu is offset by **−m × D × progress**, i.e. it slides back *against* the turn, fading out.
  - New face: the menu is offset by **+m × D × (1 − progress)**, i.e. it enters from the leading edge, fading in.
- Further input is ignored until the tip finishes.

**Neighbour previews: not shown in the current mockup**
- The code defines them but never draws them:
  - right face: next page title
  - left face: previous page title
  - bottom face: next value
  - top face: previous value
  - style: 35% alpha, 600, ≤40 canvas
- The product film does show them. **Decided (H3): off**, matching the mockup.

**Leaving**
- **Save:** hold again for **0.8 s** with no swipe (same ring), or press Enter or the Done button in the mockup.
  - Haptic **28 ms**, a pulse, and the landing-style flash.
  - The menu fades out over 0.3 s (scale 1 → 1.12, ring flashing out).
  - Then the **success screen** shows on that face (C4), and the other faces show the wake label for 2.2 s.
- **Discard:** any of the following closes the menu without saving (the wake label still shows for 2.2 s, unless a throw follows):
  - Escape,
  - a throw or shake,
  - changing view or docking,
  - **25 s** with no input.
- **Restart** (holding while "Restart" is selected): screens go black for **0.8 s**, then boot (C1). Any count or die change made in the same menu session is **not** saved.

### C4. Success screen (after saving)
- **Duration:** about **1.25 s** on the menu face.
  - Alpha rises over 0.15 s and holds.
  - Fades out from 0.9 s to 1.25 s.
- **Check mark:** a polyline (−14, −40) → (−4, −30) → (16, −52) in canvas units, stroke 5 (2.9 px), round caps. It draws itself from 0.05 s to 0.35 s.
- **Setup label:** 700, `fitPx(label, 50)`, at y +6 (px y 51.5).
- **"Ready to roll":** 600, 15 → 8.7 px, 75% alpha, at y +48 (px y 75.8).

### C5. Shake, throw, tumble

**Shake** (the mockup's Throw button held down; on hardware, the die shaken in the hand)
- **Charge** rises 0 → 1 over 1.6 s.
- **Smoke:** held smoke particles are added up to **380 × charge** (at most 14 per frame) on every face except the face down.
- **Agitation** = 3 + 3 × charge. Particles are jostled and sloshed, and held smoke is pulled toward face centers (D2).
- **Visual jitter:** random rotation ±0.06 rad per frame, x jitter ±0.03 u, bob |sin(23t)| × 0.08 u.
- **Put down without throwing:** the smoke is released and fades (particle life jumps to 35%).

**Throw** (release)
- A quick press also throws; there is no minimum shake.
- Top the smoke up to 380 (held).
- **Charge is set to 1**, whatever the shake reached.
- Pick the values with the RNG, **now** (see C6).
- Start the tumble.

**Tumble**
- Duration **1.25 s**.
- **Orientation:** slerp from the current orientation to a random target (random multiples of 90° on each axis, then a random yaw of ±0.35 rad), ease-out cubic, plus an extra spin of 4π × (1 − e) about a random axis.
- **Height:** |sin(2.5πu)| × (1 − u)² × 1.6 u.
- **Smoke during the tumble:** agitation = 5, slosh = 0.

**Landing**
- Face flash: `#E8ECF2` at 35% alpha, fading over 0.25 s.
- Held smoke is released to drain: life resets, max life 2.0–3.4 s, and it lingers banked for 0.6 s.
- Smoke is topped up by 380 × (1 − 0.8 × charge), also lingering 0.6 s. Because the throw forces charge to 1, this is always **76** particles (30 with reduced motion).
- **36 embers** are added.
- Reveal starts **0.35 s** after landing.

### C6. Results
- **Values:** numeric dice are uniform 1..N. Pass the Pot faces:

  | Face | Probability |
  |---|---|
  | ← (pass left) | 1/6 |
  | P (pot) | 1/6 |
  | → (pass right) | 1/6 |
  | • (keep) | 1/2 |

- **Where it shows:** every face **except the one facing down**.
  - Fades in over **0.9 s** from reveal.
  - Dims at **reveal + 7 s**, fading over 1.2 s.
  - Touching a dimmed die brings the result back for 4 s. In the mockup, any press on the canvas counts, and the result returns at full brightness instantly (no fade-in).
- **Numeric layout** (canvas → px)
  - **Big number:** 700.
    - Base size by number of characters: 1–2 → 94 (54 px); 3 → 66 (38 px); 4 → 52 (30 px); 5–6 → 40 (23 px); more → 32 (18.5 px).
    - With parts: × 0.85, at y 0 (px 48).
    - Without parts: at y −12 (px 41.1).
  - **Parts line** (pools only, when "a+b+c" is ≤18 characters): 600, 17 (9.8 px), or 14 (8.1 px) if longer than 10 characters; 70% alpha; at y −60 (px 13.3).
  - **Label:** 600, `fitPx(label, 22)` ≤12.7 px, at y +56 with parts or +54 without (px 80.4 / 79.2). The text is the setup label, e.g. `2d20`.
- **Pass the Pot layout:** glyphs in a row at y −12.
  - Size per glyph: 64 for 1 die, 50 for 2, 40 for 3 (37 / 29 / 23 px); gap = 1.15 × size.

  | Glyph | Drawing |
  |---|---|
  | ← / → | line from −0.36·sz to +0.36·sz, arrowhead 0.2·sz, stroke 0.1·sz |
  | • | dot of radius 0.13·sz |
  | P (pot) | stroke 0.09·sz: rim line ±0.44·sz at y −0.05·sz, bowl arc of radius 0.36·sz below it, filled dot of radius 0.11·sz at y −0.32·sz above the rim |

- **Max roll** (every die equals N, N > 2):
  - Label becomes `max 2d20`.
  - **Gold ring:** 120 gold particles circling the side faces (D3).
- **Dud** (every die equals 1):
  - Text gray `#8A8C90`, label `dud`.
  - **36 fizzle** particles on the top face.
- Pass the Pot has no max or dud.
- **Timing rule:** the max and dud effects start **only after every smoke and ember particle is gone**, and then keep the result lit for at least 5 more seconds. This rule covers the max and dud *effects* only: the normal result fades in from 0.35 s after landing, while the landing smoke is still draining. This is intended (decision H5).

### C7. Docked (Nest, and the case in the mockup)
- **Orientation:** the die sits upright (identity orientation). Particles, results and wake labels are cleared.
- **Top face: charging screen**
  - **Fill:** a liquid fill at 28% alpha rising from the bottom to the battery level, with a wavy top edge: amplitude 4 canvas (2.3 px), spatial frequency 0.09 per canvas unit, speed 2.2 rad/s. When the battery level is unknown, the fill cycles 25→100%.
  - **Percentage:** 700, 52 (30 px) at y −8 (px 43.4).
  - **Status word** "charging" or "battery": 600, 20 (11.6 px) at y +38 (px 70).
- **Side faces (Nest only): analog clock**
  - Radius R = 71 canvas (41 px).
  - 12 ticks: majors every 3 hours, from R−12 to R, stroke 4, 90% alpha; minors from R−7 to R, stroke 2.5, 50% alpha.
  - Hands:

    | Hand | Length | Stroke | Alpha |
    |---|---|---|---|
    | Hour | 0.52 R | 6 | 100% |
    | Minute | 0.8 R | 4 | 100% |
    | Second (smooth) | 0.86 R | 1.8 | 70% |

    Each hand has a 6-unit tail. Center dot radius 4.5. Glow 6.
- **Hardware additions** (not in the mockup): dim, shift the clock a pixel now and then, and turn off at night or on a timeout to avoid image retention.

### C8. Haptics (mockup = `navigator.vibrate` plus a visual pulse)
| Event | Pattern |
|---|---|
| Menu open | [18, 40, 26] ms, plus die pulse |
| Tip | 10 ms |
| Save (hold or Done) | 28 ms, plus pulse and flash |

**Planned on hardware, not in the mockup:** a rumble while shaking and a landing thunk.

---

## Part D: Smoke and particle system (exact mockup model)

### D1. Space
- Particles live on the **unit cube surface**: coordinates in [−1, 1], where each face is ±1 on its own axis.
- The lit area is **±0.648** (83/128) on that face.
- Each frame, the position is re-projected onto the cube (divide by the largest absolute component).
- When a particle crosses onto a new face, its velocity is re-tangented and it continues. Particles near an edge are also drawn onto the neighbouring face so smoke wraps continuously.
- **Drawing:** additive ("lighter") radial sprites.
  - Sprite gradient: full color at 0, 45% at 0.45, 0 at the edge.
  - Radius `size` in canvas units (× 0.5783 → px).

### D2. Per-frame update
Terms used below:
- `u` is gravity "up" in die-local coordinates.
- `n` is the particle's face normal.
- `t` is time and `s` a per-particle seed.

1. **Buoyancy:** `v += u * buoy * dt`, where `buoy` is:

   | Particle state | buoy |
   |---|---|
   | held | −0.35 |
   | lingering | −0.25 |
   | smoke | −1.3 |
   | ember | −0.8 |
   | gold, fizzle | 0 |

   Negative values fall along the die surface "downhill", which reads as smoke rolling off the edges.
2. **Held or lingering smoke banks toward the face center:** `v -= tangential(p) * 2.4 * dt`.
3. **Held smoke while shaking:** add random noise of `agitate * dt * 6` on each axis, plus `slosh * agitate * dt`.
   - `slosh` = (sin 17t, cos 13t, sin(11t+1)) × 2.5 in world space, converted to local.
4. **Top face** (not held, not gold or fizzle): push outward from the face center at `0.9 * dt`.
5. **Gold:** velocity eases toward `normalize(u × p) * 1.6`, a horizontal orbit around the die, as `v = lerp(v, target, min(1, 2.5·dt))`.
6. **All other particles:**
   - curl noise:
     - `vx += sin(3.3 py + 1.7 t + s) * 0.9 dt`
     - `vy += sin(3.1 pz + 1.9 t + 1.3 s) * 0.9 dt`
     - `vz += sin(2.9 px + 1.5 t + 0.7 s) * 0.9 dt`
   - then damping `v *= 1 − k·dt`, with k = 3 (fizzle), 1.6 (held) or 0.7 (otherwise).
7. Remove the normal component of `v`, integrate, and re-project onto the cube.
8. **Held particles don't age:** life is clamped to 20% of max.
9. **Crossing an edge:** with `n` the old face normal, `n'` the new one and `c = v·n'`: `v -= n'·c + n·c`.

The frame time step is capped at **dt ≤ 0.05 s**.

### D3. Kinds

| Kind | Sprite color | Life (s) | Size (canvas radius) | Alpha | Growth |
|---|---|---|---|---|---|
| smoke | (214, 219, 226) | 2.2–3.6 (top spawn 2.4–3.8) | 26–46 | fade in 0.12 s × (1 − life)^0.8 × **0.55** | × (1 + 1.4 · life) |
| ember | (246, 247, 249) | 0.7–1.3 | 7–14 | same fade × 0.9 | none |
| gold | (255, 255, 255) | 2.0–2.8 | 12–22 | sin(π · life)^0.8 × 0.9 | none |
| fizzle | (120, 122, 126) | 2.4–3.8 | 26–46 | fade × 0.3 | × (1 + 1.4 · life) |

`life` in the formulas is normalized (age ÷ max life).

The mockup also has "side-face" variants for smoke (life 2.2–3.5, size 20–38) and embers (life 0.9–1.6, size 8–16), but nothing spawns smoke or embers that way; only gold uses the side-face path.

**Spawn modes**
- **all:** a random face other than the face down; position bunched toward the center (a sum of 3 uniforms, ±0.85); small random velocity (±0.25).
- **top:** on the up face, bunched within ±0.95, with an outward velocity of 0.15–0.6 (embers 0.6–1.4).
- **ring:** on side faces, near mid-height (±0.175), with the gold orbit velocity.

**Boot burst:** 170 smoke particles on the top face.
- Positions: radius 0.03–0.38 from center.
- Outward speed 0.3–0.9.
- Life 1.9–2.8 s, size 22–40.
- They linger banked for 0.75 s, then roll off.

**Counts**

| Event | Particles | Reduced motion |
|---|---|---|
| Shake / throw | 380 smoke | 150 |
| Landing top-up | 76 smoke (see C5) | 30 |
| Landing | 36 embers | 10 |
| Boot burst | 170 smoke | 60 |
| Max roll | 120 gold | 40 |
| Dud | 36 fizzle | 12 |

---

## Part E: Timing cheat sheet
| Constant | Value |
|---|---|
| Hold to open or save | 0.8 s (ring shows after 0.22 s) |
| Tap threshold | movement < 8 px |
| Swipe (= tip) threshold | > 36 px |
| Menu snap on open | 0.35 s |
| Menu grow-in / fade-out | 0.35 s / 0.3 s |
| Tip | 0.42 s, hop 0.12 u |
| Menu idle timeout | 25 s |
| Success screen | ~1.25 s |
| Wake label | 2.2 s (after setup changes and boot), 3 s (tap) |
| Shake charge-up | 1.6 s |
| Tumble | 1.25 s |
| Reveal delay after landing | 0.35 s |
| Result fade-in | 0.9 s |
| Result dim | 7 s after reveal, 1.2 s fade (≥5 s after a max or dud effect; a touch adds 4 s) |
| Landing flash | 0.25 s |
| Die pulse | 0.28 s, +3.5% |
| Restart blackout | 0.8 s |
| Boot | 6.2 s total |
| Panel refresh cap | 100 Hz |
| Reduced motion | tumble 0.45 s (no spin, no bounce), tip 0.15 s, fewer particles (D3), no shake jitter, no idle spin |

## Part F: Known gaps between the mockup and hardware intent
- **Neighbour previews:** implemented but unused in the mockup, shown in the film. Decided off (H3); the film should be updated.
- **Snap to the viewer on menu open:** exists only because the mockup's viewer is a fixed camera. On hardware, the menu frame is taken from the held face and gravity.
- **Haptics:** the mockup has none for shaking or landing; they are planned.
- **Low and Ultra low power modes:** proposed, not in the mockup.
  - Low: sides at ~25% brightness, 60 Hz, dim after 5 s / off after 30 s; turns on automatically at 20% battery.
  - Ultra low: top face only, 30 Hz, dim after 3 s / off after 15 s; turns on automatically at 5% battery.
- **Idle dimming and sleep on hardware:** dim after 10 s, off after 60 s, wake on pickup. The mockup only dims results.
- **Brightness setting:** display-only in the mockup.
- **Colors:** the mockup draws screen colors in `#F4F5F7` with glow. On the panel, treat that as level 14–15 with an optional 1–2 px halo.

---

## Part G: Corrections log

Differences found when this document was checked against the mockup source (2026-09-28). Each is already folded into the text above.

| # | Section | Before | Mockup (now in the text) |
|---|---|---|---|
| G1 | B3 | No wake label on the face-down screen | The mockup draws it on every face, including face-down. Decision H2 keeps the spec (dark), so here the mockup is the one to fix |
| G2 | C3 | Old content slides *along* the surface motion; new enters from −120 × (1 − p) | Signs reversed: old −m·D·p, new +m·D·(1 − p) |
| G3 | C5 | Landing top-up 380 × (1 − 0.8 × charge) varies with the shake | The throw sets charge to 1, so it is always 76 (30 reduced) |
| G4 | C5 | Agitation only described while shaking | During the tumble, agitation = 5 and slosh = 0 |
| G5 | C2, C3 | Wake label after a save | Also after any discard |
| G6 | C3 | Restart: blackout, then boot | Restart also drops unsaved count/die changes |
| G7 | C6 | "Nothing readable should appear through smoke" | Applies to max/dud effects only; normal results fade in while smoke drains (kept, decision H5) |
| G8 | D3 | Smoke and ember "sides" sizes and lives | Unused; top-spawned smoke lives 2.4–3.8 s |
| G9 | D3, E | Reduced motion: 150 smoke | Full reduced-motion counts added |
| G10 | B1 | Rounded-corner mask is a pipeline step | It is the glass's ink mask; the simulator's glass renderer applies it, not the firmware |
| G11 | Conventions, A2 | Canvas spans "the whole face"; lit area 17.26 mm | Canvas spans ±1 scene unit (26.5 mm); the mockup draws the lit area at 17.18 mm |
| G12 | A5, C2, C3, C6, D2 | Unspecified | Portrait = aspect < 0.75; tap definition; menu open clears particles and results; dim restore is instant and triggered by any press; gold eases at 2.5/s; edge-crossing formula; dt ≤ 0.05 s; Pass the Pot has no max/dud; a quick press throws |
| G13 | A5 | — | Note on three.js r128 colour and light semantics when porting |

Everything else was checked and matches: all canvas → panel-px conversions, timings, the boot sequence (the top face's start value of 6 is an explicit override of the per-face table), the menu layout, the particle formulas, and every die, Nest and case dimension.

## Part H: Decisions

Decided 2026-09-28.

| # | Question | Decision |
|---|---|---|
| H1 | **Smoke rendering.** Part D is a live, gravity-reactive particle system, which can't be baked into frame sequences. It conflicted with the earlier "pre-rendered smoke in 64 MB QSPI" decision. | **Live particle simulation in firmware.** "Pre-rendered" now means *assets* in QSPI: smoke sprite stamps and Space Grotesk bitmaps. A theme is a sprite set plus parameters, not a video. On hardware the smoke layer is drawn at half resolution (48×48) to fit the frame budget (late-life sprites reach ~60 px radius; 380 of them at full resolution is ~1M pixel blends per frame). The simulator may draw it at full resolution. |
| H2 | **Face-down wake label** | **Dark.** Departs from the mockup (G1); the mockup should be updated. |
| H3 | **Neighbour previews** in the menu | **Off**, as in the mockup. The product film should be updated. |
| H4 | **Fidelity target** for firmware rendering | **Native 96×96 rendering, compared against captured mockup frames within a tolerance.** Pixel-identical output isn't a goal: the firmware can't afford the mockup's 256×256 canvas-and-downsample pipeline. |
| H5 | **Normal results through smoke** | **Keep the mockup:** the result fades in from 0.35 s after landing while smoke drains. Only max/dud effects wait for clear air. |
| H6 | **Pass the Pot in the signed roll** | **Sign the raw d6 value** (1 → ←, 2 → P, 3 → →, 4–6 → •), exactly as the mockup draws it. Requires roll format v2 and a 10-dice limit. |
| H7 | **Menu frame on hardware when the held face points up.** "Front" and "up" coincide, so right/up are undefined. | **Hardware question, still open.** Doesn't block the simulator, where the mockup's snap becomes a camera move instead of a die rotation. |
