# Mockup capture

Golden frames for the firmware renderer ([SIM_SPEC](../../docs/SIM_SPEC.md), decision H4).

`capture.mjs` runs `mockup.html`, a pinned copy of the interactive mockup
(artifact `8Z1hGvE7k3mf5sGfaWEJ7j`, version `1790415968-5231`), in headless
Chromium and saves each face's quantized 96×96 output at fixed moments.

To make runs reproducible:
- **Assets:** three.js r128 and Space Grotesk come from npm packages (the page's
  CDN requests are intercepted), so capture works offline.
- **Randomness:** `Math.random` is seeded.
- **Time:** `performance.now`, `Date` and `requestAnimationFrame` are virtual.
  The harness steps the page exactly 60 frames per mockup second; the Nest
  clock starts at 10:08:30.
- **Battery:** the Battery API is hidden, so the mockup uses its stand-in
  value.
- **Output:** `CanvasRenderingContext2D.putImageData` is hooked to read the
  panel output.

## Output

`golden/<scenario>/`:
- `<time>.bin`: six faces in `Face` order (+X −X +Y −Y +Z −Z), 4 bits per
  pixel, high nibble first. This is the same layout as the simulator's frame
  packet without its tag byte.
- `sheet.png`: every checkpoint side by side, for eyeballing.

`golden/manifest.json` lists each scenario's inputs and checkpoint times.

| Scenario | What it covers |
|---|---|
| `boot` | Power-on pips, burst, SMOKEBOMB, wake label |
| `tap` | Tap → setup label on every face |
| `throw` | Hold 1 s to shake, release, tumble, landing, reveal, dim |
| `quickThrow` | A quick press throws without shaking |
| `passThePot` | Pass the Pot chosen and thrown |
| `menu` | Hold ring, open, tip page/value, hold to save, success |
| `menuSettings` | Every Settings item |
| `restart` | Settings → Restart: blackout, then boot |
| `nest` | Docked: charging screen and clocks |

## Running

```sh
npm install
npm run capture -w @smokebomb/mockup-capture              # all scenarios, ~10 min
npm run capture -w @smokebomb/mockup-capture -- menu tap  # just these
```

It uses Playwright's Chromium (`npx playwright install chromium`), or set
`CHROMIUM_PATH` to use another. Rendering is software GL, so capture is slow:
about a minute per scenario. CI doesn't run it; regenerate the goldens only
when the mockup or a scenario changes, and commit them.

## Caveats

- ▲ and ▼ aren't in Space Grotesk, so they come from a system fallback font
  and differ slightly between machines.
- `mockup.html` is a snapshot. When the mockup changes, replace it, bump the
  version in `capture.mjs`'s manifest line, and re-capture.
