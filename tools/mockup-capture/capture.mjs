// Captures the mockup's 96x96 panel output at fixed moments, as golden frames
// for the firmware renderer (docs/SIM_SPEC.md, decision H4).
//
// The mockup (mockup.html, a pinned copy of the artifact) runs in headless
// Chromium with:
//   - three.js r128 and Space Grotesk served from npm packages, so it runs
//     offline and identically everywhere;
//   - Math.random seeded, and time virtualised (performance.now, Date,
//     requestAnimationFrame), so every run produces the same frames;
//   - putImageData hooked, to read each face's quantized 96x96 output.
//
// Output, per scenario, in golden/<scenario>/:
//   <time>.bin       six faces in Face order, 4bpp packed (high nibble first),
//                    the same layout as the simulator's frame packet
//   sheet.png        every checkpoint side by side, for eyeballing
// plus golden/manifest.json describing scenarios, inputs and checkpoints.
//
// Usage: npm run capture -w @smokebomb/mockup-capture [-- scenario...]

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const OUT = join(here, "golden");
const FPS = 60;
// Small viewport: rendering speed only; panel output does not depend on it.
const VIEWPORT = { width: 640, height: 400 };
const SEED = 42;
// Wall clock the mockup's Nest clock shows at t = 0.
const EPOCH_MS = Date.UTC(2026, 0, 1, 10, 8, 30);

// ---------- scenarios ----------
// Each step runs at a mockup time `t` (seconds since the first frame):
//   { t, capture: "name" }     save the panels
//   { t, do: async (ctl) => }  send input
const range = (from, to, step) => {
  const out = [];
  for (let t = from; t <= to + 1e-9; t += step) out.push(Math.round(t * 1000) / 1000);
  return out;
};
const captures = (times) => times.map((t) => ({ t, capture: t.toFixed(2) }));

const SCENARIOS = {
  boot: {
    about: "Power-on: pips, top-face burst, SMOKEBOMB, then the wake label (C1, C2)",
    steps: captures([0.1, 0.25, 0.5, 0.8, 1.2, 1.6, 2.0, 2.2, 2.35, 2.5, 2.7, 3.0, 3.6, 4.2, 5.0, 5.9, 6.1, 6.5, 7.5, 8.2, 8.6]),
  },
  tap: {
    about: "Tap a screen after boot: the setup label on every face for 3 s (C2)",
    steps: [{ t: 9.0, do: (c) => c.tapDie() }, ...captures([9.1, 9.3, 9.6, 11.0, 11.8, 12.2])],
  },
  throw: {
    about: "Hold the throw button 1 s (shake), release: tumble, landing, reveal, dim (C5, C6)",
    steps: [
      { t: 9.0, do: (c) => c.pointerDown("#throw") },
      ...captures([9.2, 9.5, 9.9]),
      { t: 10.0, do: (c) => c.pointerUp() },
      ...captures([10.2, 10.6, 11.0, 11.25, 11.3, 11.45, 11.6, 11.8, 12.1, 12.5, 13.5, 15.0, 17.0, 18.6, 19.2, 19.8, 20.5]),
    ],
  },
  quickThrow: {
    about: "A quick press throws without a shake (C5)",
    steps: [
      { t: 9.0, do: (c) => c.pointerDown("#throw") },
      { t: 9.05, do: (c) => c.pointerUp() },
      ...captures([9.3, 10.4, 10.6, 11.2, 12.0]),
    ],
  },
  passThePot: {
    about: "Pass the Pot ×1: set with the die chip, then throw (C6)",
    steps: [
      { t: 9.0, do: (c) => c.click('#types button:has-text("Pass the Pot")') },
      ...captures([9.2, 10.0]),
      { t: 11.5, do: (c) => c.pointerDown("#throw") },
      { t: 12.0, do: (c) => c.pointerUp() },
      ...captures([13.6, 14.5, 16.0]),
    ],
  },
  menu: {
    about: "Hold a screen: ring, menu opens; tip left (page), tip up (value), hold to save: success (C3, C4)",
    steps: [
      { t: 9.0, do: (c) => c.pressDie() },
      ...captures([9.1, 9.3, 9.5, 9.7, 9.85, 10.0, 10.2]),
      { t: 10.3, do: (c) => c.pointerUp() },
      { t: 10.6, do: (c) => c.key("ArrowLeft") },
      ...captures([10.7, 10.85, 11.1]),
      { t: 11.2, do: (c) => c.key("ArrowUp") },
      ...captures([11.35, 11.7]),
      { t: 11.8, do: (c) => c.key("ArrowLeft") },
      { t: 12.3, do: (c) => c.key("ArrowLeft") },
      ...captures([12.8]),
      { t: 12.9, do: (c) => c.key("ArrowRight") },
      { t: 13.4, do: (c) => c.key("ArrowRight") },
      ...captures([13.9]),
      { t: 14.0, do: (c) => c.pressDie() },
      ...captures([14.3, 14.6]),
      { t: 14.85, do: (c) => c.pointerUp() },
      ...captures([14.9, 15.0, 15.2, 15.5, 15.9, 16.3, 17.2]),
    ],
  },
  menuSettings: {
    about: "Menu Settings page: each item (C3)",
    steps: [
      { t: 9.0, do: (c) => c.click("#menu") },
      { t: 9.5, do: (c) => c.key("ArrowRight") },
      ...captures([10.0]),
      ...range(10.1, 14.1, 0.5).flatMap((t, i) => [
        { t, do: (c) => c.key("ArrowUp") },
        { t: t + 0.45, capture: `item${i + 1}` },
      ]),
    ],
  },
  restart: {
    about: "Settings → Restart, hold: blackout, then boot again (C3, C1)",
    steps: [
      { t: 9.0, do: (c) => c.click("#menu") },
      { t: 9.5, do: (c) => c.key("ArrowRight") },
      ...range(10.0, 14.0, 0.5).map((t) => ({ t, do: (c) => c.key("ArrowDown") })),
      ...captures([14.5]),
      { t: 14.6, do: (c) => c.key("Enter") },
      ...captures([14.7, 15.3, 15.5, 16.0, 17.0]),
    ],
  },
  nest: {
    about: "Docked in the Nest: charging screen on top, clocks on the sides (C7)",
    steps: [{ t: 1.0, do: (c) => c.click('#views button:has-text("Nest")') }, ...captures([2.0, 3.0, 6.0])],
  },
};

// ---------- page setup ----------
const INIT = `(() => {
  // Seeded Math.random (mulberry32).
  let s = ${SEED} >>> 0;
  Math.random = () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  // Virtual time.
  let ms = 0;
  performance.now = () => ms;
  const RealDate = Date;
  Date = class extends RealDate {
    constructor(...a) { super(...(a.length ? a : [${EPOCH_MS} + ms])); }
    static now() { return ${EPOCH_MS} + ms; }
  };
  let queue = [];
  window.requestAnimationFrame = (cb) => { queue.push(cb); return queue.length; };
  window.cancelAnimationFrame = () => {};
  // Deterministic battery: no Battery API, so the mockup uses its stand-in.
  try { Object.defineProperty(navigator, "getBattery", { value: undefined }); } catch (e) {}
  // Capture each face's quantized output (drawn in face order every frame).
  let current = [];
  window.__frame = null;
  const put = CanvasRenderingContext2D.prototype.putImageData;
  CanvasRenderingContext2D.prototype.putImageData = function (img, ...rest) {
    if (this.canvas.width === 96 && img.width === 96 && img.height === 96) {
      const lv = new Uint8Array(96 * 96);
      for (let i = 0; i < lv.length; i++) lv[i] = img.data[i * 4] / 17;
      current.push(lv);
    }
    return put.call(this, img, ...rest);
  };
  window.__frames = 0;
  window.__ready = () => queue.length > 0;
  window.__step = (n) => {
    for (let i = 0; i < n; i++) {
      ms += 1000 / ${FPS};
      const q = queue; queue = []; current = [];
      q.forEach((cb) => cb(ms));
      window.__frames++;
      if (current.length === 6) window.__frame = current.map((f) => Array.from(f));
    }
    return window.__frames;
  };
})();`;

async function assets() {
  const three = await readFile(join(here, "node_modules/three/build/three.min.js")).catch(() =>
    readFile(require.resolve("three/build/three.min.js")),
  );
  const fontDir = dirname(require.resolve("@fontsource/space-grotesk/package.json"));
  // The mockup requests weights 400, 500 and 700 only (so "600" renders as 700).
  const css = (
    await Promise.all([400, 500, 700].map((w) => readFile(join(fontDir, `${w}.css`), "utf8")))
  ).join("\n");
  return { three, css, fontDir };
}

// ---------- driving ----------
async function runScenario(browser, name, scenario, files) {
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.addInitScript(INIT);
  await page.route("**/*", async (route) => {
    const url = route.request().url();
    if (url.startsWith("http://mockup.local/")) {
      return route.fulfill({ contentType: "text/html", body: await readFile(join(here, "mockup.html")) });
    }
    if (url.includes("cdnjs.cloudflare.com") && url.endsWith("three.min.js")) {
      return route.fulfill({ contentType: "text/javascript", body: files.three });
    }
    if (url.startsWith("https://fonts.googleapis.com/css2")) {
      return route.fulfill({ contentType: "text/css", body: files.css });
    }
    const font = url.match(/\/files\/(space-grotesk-[\w-]+\.woff2?)$/);
    if (font) return route.fulfill({ contentType: "font/woff2", body: await readFile(join(files.fontDir, "files", font[1])) });
    return route.abort();
  });
  await page.goto("http://mockup.local/");
  await page.waitForFunction(() => window.__ready());

  // Mockup time is (frames - 1) / FPS: the first frame's delta is zero.
  let frames = 0;
  const advanceTo = async (t) => {
    const target = Math.round(t * FPS) + 1;
    if (target > frames) frames = await page.evaluate((n) => window.__step(n), target - frames);
  };
  const center = async (selector) => {
    const b = await page.locator(selector).boundingBox();
    return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
  };
  // The die's centre projects slightly above the viewport centre (the camera
  // looks 0.35 K below it); pressing there always hits a screen.
  const dieCentre = { x: VIEWPORT.width / 2, y: VIEWPORT.height * 0.425 };
  const ctl = {
    pointerDown: async (selector) => {
      const p = await center(selector);
      await page.mouse.move(p.x, p.y);
      await page.mouse.down();
    },
    pointerUp: () => page.mouse.up(),
    pressDie: async () => {
      await page.mouse.move(dieCentre.x, dieCentre.y);
      await page.mouse.down();
    },
    tapDie: async () => {
      await page.mouse.move(dieCentre.x, dieCentre.y);
      await page.mouse.down();
      await page.mouse.up();
    },
    click: async (selector) => {
      const p = await center(selector);
      await page.mouse.click(p.x, p.y);
    },
    key: (k) => page.keyboard.press(k),
  };

  const dir = join(OUT, name);
  await mkdir(dir, { recursive: true });
  const shots = [];
  for (const step of [...scenario.steps].sort((a, b) => a.t - b.t)) {
    await advanceTo(step.t);
    if (step.do) await step.do(ctl);
    if (step.capture) {
      const faces = await page.evaluate(() => window.__frame);
      const packed = Buffer.alloc(6 * 4608);
      faces.forEach((lv, f) => {
        for (let i = 0; i < lv.length; i += 2) packed[f * 4608 + i / 2] = (lv[i] << 4) | lv[i + 1];
      });
      const file = `${step.capture}.bin`;
      await writeFile(join(dir, file), packed);
      shots.push({ t: step.t, name: step.capture, file, faces });
    }
  }
  await writeFile(join(dir, "sheet.png"), await contactSheet(page, name, shots));
  await page.close();
  if (errors.length) throw new Error(`${name}: page errors: ${errors.join("; ")}`);
  return {
    about: scenario.about,
    inputs: scenario.steps.filter((s) => s.do).map((s) => ({ t: s.t, input: s.do.toString().replace(/^\(c\) => c\./, "") })),
    checkpoints: shots.map(({ t, name: n, file }) => ({ t, name: n, file })),
  };
}

async function contactSheet(page, title, shots) {
  const url = await page.evaluate(
    ({ title, shots }) => {
      const S = 2, W = 96 * S, GAP = 6, LABEL = 70, TOP = 28;
      const c = document.createElement("canvas");
      c.width = LABEL + 6 * (W + GAP);
      c.height = TOP + shots.length * (W + GAP);
      const x = c.getContext("2d");
      x.fillStyle = "#16161b";
      x.fillRect(0, 0, c.width, c.height);
      x.fillStyle = "#e8e8ee";
      x.font = "14px sans-serif";
      x.fillText(`${title}   faces: +X −X +Y −Y +Z −Z`, 8, 18);
      shots.forEach((s, row) => {
        const y = TOP + row * (W + GAP);
        x.fillStyle = "#8a8a99";
        x.fillText(`t=${s.t.toFixed(2)}`, 8, y + 16);
        s.faces.forEach((lv, f) => {
          const img = x.createImageData(96, 96);
          lv.forEach((l, i) => {
            img.data[i * 4] = img.data[i * 4 + 1] = img.data[i * 4 + 2] = l * 17;
            img.data[i * 4 + 3] = 255;
          });
          const tmp = document.createElement("canvas");
          tmp.width = tmp.height = 96;
          tmp.getContext("2d").putImageData(img, 0, 0);
          x.imageSmoothingEnabled = false;
          x.drawImage(tmp, LABEL + f * (W + GAP), y, W, W);
        });
      });
      return c.toDataURL("image/png");
    },
    { title, shots },
  );
  return Buffer.from(url.split(",")[1], "base64");
}

// ---------- main ----------
const only = process.argv.slice(2);
const selected = Object.entries(SCENARIOS).filter(([n]) => !only.length || only.includes(n));
const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM_PATH || undefined,
  args: ["--use-gl=swiftshader", "--enable-unsafe-swiftshader", "--font-render-hinting=none"],
});
const files = await assets();
const manifestPath = join(OUT, "manifest.json");
const manifest = JSON.parse(await readFile(manifestPath, "utf8").catch(() => "{}"));
manifest.mockup = "tools/mockup-capture/mockup.html (artifact 8Z1hGvE7k3mf5sGfaWEJ7j, version 1790415968-5231)";
manifest.fps = FPS;
manifest.seed = SEED;
manifest.viewport = VIEWPORT;
manifest.format = "6 faces × 4608 bytes, Face order (+X −X +Y −Y +Z −Z), 4bpp packed high nibble first";
manifest.scenarios ??= {};
for (const [name, scenario] of selected) {
  manifest.scenarios[name] = await runScenario(browser, name, scenario, files);
  console.log(`${name}: ${manifest.scenarios[name].checkpoints.length} checkpoints`);
}
await browser.close();
await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
