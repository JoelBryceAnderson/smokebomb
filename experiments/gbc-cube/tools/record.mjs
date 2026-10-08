// Drive the browser simulator through a scripted walk and save screenshots
// and a video: walking north, rolling the cube onto new faces mid-walk, a
// menu, a battle.
//
//   gbc-cube serve --demo &              (or with your ROM, past the title)
//   PLAYWRIGHT=$(npm root -g)/playwright/index.mjs node tools/record.mjs OUT_DIR
//
// It uses the page's `window.gbc` hooks (web/src/main.ts) rather than
// synthesising key presses, so timings are steady.

import { mkdirSync } from "node:fs";

const out = process.argv[2] ?? "record";
const url = process.env.GBC_CUBE_URL ?? "http://localhost:3100";
const { chromium } = await import(process.env.PLAYWRIGHT ?? "playwright");
mkdirSync(out, { recursive: true });

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM ?? undefined,
  args: ["--use-gl=angle", "--use-angle=swiftshader", "--enable-unsafe-swiftshader"],
});
const size = { width: 1280, height: 720 };
const context = await browser.newContext({ viewport: size, recordVideo: { dir: out, size } });
const page = await context.newPage();
await page.goto(url);
await page.waitForFunction(() => window.gbc && window.gbc.frames() > 30);

const wait = (ms) => page.waitForTimeout(ms);
const gbc = (fn, ...args) => page.evaluate(([fn, args]) => window.gbc[fn](...args), [fn, args]);
let n = 0;
const shot = async (name) => {
  n += 1;
  const path = `${out}/${String(n).padStart(2, "0")}-${name}.png`;
  await page.screenshot({ path });
  console.log(path);
};
const FACES = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
const upFace = async () => FACES.indexOf((await gbc("status")).up);
const lean = 0.42; // ~24°: walking, well short of rolling

await gbc("view", 2.4, 2.6, 3.4);
await wait(800);
await shot("start");

// Walk north by tilting: the north edge (away from the camera) dips.
await gbc("tilt", 0, -lean);
await wait(1500);
await shot("walking-north");

// Mid-walk, tip it right over to the east: the west face comes up, and
// the map rolls with it so north stays north.
await gbc("roll", "e");
await wait(700);
await shot("rolled-east-mid-walk");
await wait(1200);
await shot("still-walking-north");

// And over to the north.
await gbc("roll", "n");
await wait(700);
await gbc("tilt", lean, 0); // now walk east
await wait(1400);
await shot("rolled-north-walking-east");
await gbc("tilt", 0, 0);
await wait(600);

// A corner view: the map reads straight across the up face's edges.
await gbc("view", 1.6, 2.0, 1.8);
await wait(700);
await shot("corner-view");
await gbc("view", 2.4, 2.6, 3.4);
await wait(500);

// Long-press the up face: Start.
const up = await upFace();
await gbc("touch", 1 << up);
await wait(900);
await gbc("touch", 0);
await wait(500);
await gbc("view", 0.3, 1.6, 3.6); // look at the front face
await wait(700);
await shot("start-menu-front-face");
// Tap a side face: B closes it.
await gbc("touch", 1 << ((up + 2) % 6));
await wait(120);
await gbc("touch", 0);
await wait(600);

// Shake: Select (the demo cart starts a battle on Select).
await gbc("shake");
await wait(2500);
await gbc("touch", 1 << (await upFace())); // A through the intro text
await wait(120);
await gbc("touch", 0);
await wait(800);
await gbc("view", 1.2, 2.3, 3.4);
await wait(700);
await shot("battle");
await gbc("keys", 0x02); // B: run
await wait(150);
await gbc("keys", 0);
await wait(1500);
await gbc("keys", 0x01);
await wait(150);
await gbc("keys", 0);
await wait(1000);
await shot("back-on-the-map");

await context.close();
await browser.close();
console.log(`video in ${out}/`);
