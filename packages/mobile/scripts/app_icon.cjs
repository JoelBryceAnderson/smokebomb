#!/usr/bin/env node
// Draws the Sugarcube app icon and writes every iOS and Android size.
//
// The icon is an old sugar box: aged cream paper, a sunburst, a red and navy
// frame and a ribbon with "Sugarcube" in Pacifico (the face the die writes
// its name in). On it stands the mascot, a rubber-hose sugar cube giving a
// thumbs up.
//
// The art is SVG, rendered by Chromium through Playwright:
//   npm i -g playwright   (or use any install that `require` can find)
//   node packages/mobile/scripts/app_icon.cjs

const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const MOBILE = path.resolve(__dirname, "..");
const REPO = path.resolve(MOBILE, "../..");
const FONT = path.join(REPO, "packages/firmware/assets/fonts/pacifico-latin-400-normal.woff");
const IOS_SET = path.join(MOBILE, "iosApp/iosApp/Assets.xcassets/AppIcon.appiconset");
const ANDROID_RES = path.join(MOBILE, "composeApp/src/androidMain/res");

const C = {
  paper: "#F4E7C9",
  paperDark: "#E2C892",
  ray: "#EED7A1",
  red: "#C63A2E",
  redDark: "#8E2620",
  navy: "#1F2B4D",
  ink: "#1A1410",
  sugar: "#FFFDF5",
  sugarTop: "#FFFFFF",
  sugarSide: "#E8DFC9",
  cheek: "#F29C93",
  mouth: "#6E1B17",
  tongue: "#E8706A",
  gold: "#E7A93B",
};

// --- Pieces, all on a 1024 canvas -----------------------------------------

function paper() {
  const rays = [];
  const n = 28;
  const [cx, cy] = [512, 470];
  for (let i = 0; i < n; i += 2) {
    const a0 = (i / n) * Math.PI * 2;
    const a1 = ((i + 1) / n) * Math.PI * 2;
    const r = 900;
    rays.push(
      `M${cx},${cy} L${cx + r * Math.cos(a0)},${cy + r * Math.sin(a0)} L${cx + r * Math.cos(a1)},${cy + r * Math.sin(a1)} Z`,
    );
  }
  return `
    <rect width="1024" height="1024" fill="${C.paper}"/>
    <path d="${rays.join(" ")}" fill="${C.ray}"/>
    <rect width="1024" height="1024" fill="url(#age)"/>
    <rect width="1024" height="1024" filter="url(#grain)" opacity="0.35"/>`;
}

function frame() {
  return `
    <rect x="44" y="44" width="936" height="936" rx="196" fill="none" stroke="${C.red}" stroke-width="22"/>
    <rect x="76" y="76" width="872" height="872" rx="168" fill="none" stroke="${C.navy}" stroke-width="6"/>`;
}

// Where the thumbs-up glove's cuff opens, in mascot coordinates: the arm ends here.
const GLOVE = { x: 822, y: 336, tilt: -12 };
const CUFF = [
  GLOVE.x - 100 * Math.sin((GLOVE.tilt * Math.PI) / 180),
  GLOVE.y + 100 * Math.cos((GLOVE.tilt * Math.PI) / 180),
];

function glove() {
  // A puffy cartoon glove giving a thumbs up: one soft outline for the fist
  // and the bent thumb, knuckle bumps down the curled fingers, a flared cuff
  // with a rolled edge, and a few "ta-da" ticks off the thumb.
  const { x, y, tilt } = GLOVE;
  const fist = `M-46,-10
    C-62,-48 -54,-98 -30,-112 C-12,-122 6,-110 2,-90 C-2,-70 -10,-52 -2,-38
    C14,-48 36,-46 46,-30 C64,-26 66,-4 50,0 C68,4 68,26 50,28
    C66,34 62,54 40,56 C18,64 -30,62 -44,48 C-62,34 -62,6 -46,-10 Z`;
  return `
    <g transform="translate(${x},${y}) rotate(${tilt})" stroke="${C.ink}" stroke-width="12" stroke-linejoin="round" stroke-linecap="round">
      <path class="solid-w" d="M-36,52 C-46,70 -54,84 -52,94 C-20,108 30,108 54,94 C54,84 46,68 40,52 Z" fill="${C.sugarTop}"/>
      <path d="M-48,82 C-16,94 26,94 50,82" fill="none" stroke-width="8"/>
      <path class="solid-w" d="${fist}" fill="${C.sugarTop}"/>
      <path d="M50,0 C40,4 30,2 20,-4 M50,28 C40,32 30,30 20,24 M-2,-38 C-14,-30 -26,-28 -36,-32" fill="none" stroke-width="8"/>
      <path d="M-24,40 C-10,46 6,46 18,40" fill="none" stroke-width="6"/>
      <path d="M-58,-100 L-78,-108 M-46,-132 L-58,-148 M14,-118 L30,-130" fill="none" stroke-width="9"/>
    </g>`;
}

function hipGlove(x, y) {
  return `
    <g transform="translate(${x},${y})" stroke="${C.ink}" stroke-width="12" stroke-linejoin="round">
      <ellipse class="solid-w" cx="0" cy="0" rx="40" ry="34" fill="${C.sugarTop}"/>
      <path d="M-6,-18 Q8,-4 -6,14" fill="none" stroke-width="7" stroke-linecap="round"/>
    </g>`;
}

function sparkle(x, y, r, fill) {
  const w = r * 0.28;
  return `<path class="solid" d="M${x},${y - r} Q${x + w},${y - w} ${x + r},${y} Q${x + w},${y + w} ${x},${y + r} Q${x - w},${y + w} ${x - r},${y} Q${x - w},${y - w} ${x},${y - r} Z" fill="${fill}"/>`;
}

// A closed path through `pts` ([x, y, radius]) with each corner rounded.
function roundedPoly(pts) {
  const n = pts.length;
  const toward = (a, b, d) => {
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
    return [a[0] + ((b[0] - a[0]) * d) / len, a[1] + ((b[1] - a[1]) * d) / len];
  };
  let d = "";
  for (let i = 0; i < n; i++) {
    const [p, prev, next] = [pts[i], pts[(i + n - 1) % n], pts[(i + 1) % n]];
    const a = toward(p, prev, p[2]);
    const b = toward(p, next, p[2]);
    d += `${i ? "L" : "M"}${a[0].toFixed(1)},${a[1].toFixed(1)} Q${p[0]},${p[1]} ${b[0].toFixed(1)},${b[1].toFixed(1)} `;
  }
  return d + "Z";
}

// The cube's edge rounding. The die's is 2.5 mm on 34 mm (SIM_SPEC); the
// mascot exaggerates it so it still reads at launcher size.
const R = 46;

function mascot() {
  // Cube: front face, top and right side in a loose three-quarter view, with
  // the outer corners rounded and the three inner edges meeting at (650, 330).
  const front = roundedPoly([[330, 330, R], [650, 330, 0], [650, 640, R], [330, 640, R]]);
  const top = roundedPoly([[330, 330, R], [398, 268, R * 0.7], [718, 268, R], [650, 330, 0]]);
  const side = roundedPoly([[650, 330, 0], [718, 268, R], [718, 578, R * 0.7], [650, 640, R]]);
  const outline = roundedPoly([
    [330, 330, R], [398, 268, R * 0.7], [718, 268, R], [718, 578, R * 0.7], [650, 640, R], [330, 640, R],
  ]);
  // Inner edges fade out where they reach a rounded corner.
  const edges = `M${330 + R},330 L650,330 L${718 - R * 0.55},${268 + R * 0.5} M650,330 L650,${640 - R}`;
  const grains = [];
  // Granulated sugar: little specks on each face.
  let seed = 11;
  const rnd = () => ((seed = (seed * 9301 + 49297) % 233280) / 233280);
  for (let i = 0; i < 40; i++) {
    const x = 350 + rnd() * 280;
    const y = 350 + rnd() * 270;
    if (Math.hypot(x - 490, y - 490) < 125) continue; // keep the face clean
    grains.push(`<rect x="${x.toFixed(1)}" y="${y.toFixed(1)}" width="7" height="7" transform="rotate(${(rnd() * 90).toFixed(0)} ${x.toFixed(1)} ${y.toFixed(1)})"/>`);
  }
  // Pie-cut eyes: black ovals with a wedge of white taken out, the 1930s way.
  const eye = (cx) => `
    <g>
      <ellipse class="solid" cx="${cx}" cy="448" rx="27" ry="44" fill="${C.ink}"/>
      <path class="cut" d="M${cx + 4},${448 - 4} L${cx + 30},${448 - 34} L${cx + 30},${448 - 4} Z" fill="${C.sugar}"/>
    </g>`;
  return `
    <g class="mascot">
      <!-- legs and shoes -->
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" fill="none">
        <path d="M440,630 Q430,690 418,724"/>
        <path d="M545,630 Q560,690 578,724"/>
      </g>
      <ellipse class="solid" cx="398" cy="736" rx="56" ry="26" fill="${C.ink}"/>
      <ellipse class="solid" cx="602" cy="736" rx="56" ry="26" fill="${C.ink}"/>
      <ellipse class="cut" cx="380" cy="726" rx="16" ry="7" fill="#FFFFFF" opacity="0.5"/>
      <ellipse class="cut" cx="620" cy="726" rx="16" ry="7" fill="#FFFFFF" opacity="0.5"/>
      <!-- the cube -->
      <path class="solid-w" d="${side}" fill="${C.sugarSide}"/>
      <path class="solid-w" d="${top}" fill="${C.sugarTop}"/>
      <path class="solid-w" d="${front}" fill="${C.sugar}"/>
      <path d="${edges}" fill="none" stroke="${C.ink}" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
      <path d="${outline}" fill="none" stroke="${C.ink}" stroke-width="14" stroke-linejoin="round"/>
      <g class="grain" fill="${C.sugarSide}">${grains.join("")}</g>
      <!-- arm on the hip (left), thumbs-up arm (right) -->
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" fill="none">
        <path d="M334,470 C250,462 238,560 300,582"/>
        <path d="M716,450 C790,452 ${CUFF[0]},${CUFF[1] + 30} ${CUFF[0]},${CUFF[1] - 4}"/>
      </g>
      ${hipGlove(306, 584)}
      ${glove()}
      <!-- face -->
      ${eye(445)}
      ${eye(540)}
      <ellipse class="cheek" cx="398" cy="520" rx="28" ry="17" fill="${C.cheek}"/>
      <ellipse class="cheek" cx="588" cy="520" rx="28" ry="17" fill="${C.cheek}"/>
      <path class="solid" d="M418,512 Q492,612 566,512 Q492,534 418,512 Z" fill="${C.mouth}" stroke="${C.ink}" stroke-width="10" stroke-linejoin="round"/>
      <path class="cut" d="M455,560 Q492,548 530,560 Q494,590 455,560 Z" fill="${C.tongue}"/>
      <path d="M408,506 Q418,500 426,510 M558,510 Q566,500 576,506" stroke="${C.ink}" stroke-width="8" stroke-linecap="round" fill="none"/>
      <!-- shine marks -->
      ${sparkle(248, 330, 30, C.gold)}
      ${sparkle(290, 268, 16, C.gold)}
      ${sparkle(770, 560, 22, C.gold)}
    </g>`;
}

// The ribbon: an arched band from x0 to x1 whose middle rises by `lift`,
// `h` deep, with notched tails tucked behind each end.
const RIBBON = { x0: 158, x1: 866, y: 850, lift: 92, h: 172 };

function banner() {
  const { x0, x1, y, lift, h } = RIBBON;
  const mid = (x0 + x1) / 2;
  // A quadratic's peak is half its control point's height, so the control
  // point sits at 2 * lift.
  const arc = (dy) => `${x0},${y + dy} Q${mid},${y + dy - 2 * lift} ${x1},${y + dy}`;
  const top = -h / 2;
  const bot = h / 2;
  const tail = (sx) => {
    // Mirror about the centre for the right tail.
    const X = (v) => (sx > 0 ? v : 2 * mid - v);
    return `
      <path class="solid" d="M${X(x0 + 40)},${y + top + 40} L${X(x0 - 64)},${y + top + 52} L${X(x0 - 28)},${y + 24}
        L${X(x0 - 60)},${y + bot + 8} L${X(x0 + 46)},${y + bot + 14} Z" fill="${C.redDark}"/>
      <path class="solid" d="M${X(x0)},${y + bot} L${X(x0 + 46)},${y + bot + 14} L${X(x0 + 40)},${y + bot - 14} Z" fill="${C.navy}"/>`;
  };
  return `
    <defs><path id="ribbon-line" d="M${arc(0)}"/></defs>
    <g class="banner" stroke="${C.navy}" stroke-width="10" stroke-linejoin="round">
      ${tail(1)}
      ${tail(-1)}
      <path class="solid" d="M${arc(top)} L${x1},${y + bot} Q${mid},${y + bot - 2 * lift} ${x0},${y + bot} Z" fill="${C.red}"/>
      <path d="M${arc(top + 14)} M${arc(bot - 14)}" fill="none" stroke="${C.paper}" stroke-width="4" opacity="0.7"/>
    </g>
    <text class="word" font-family="Pacifico" font-size="108" dy="22" text-anchor="middle"
      fill="${C.paper}" stroke="${C.navy}" stroke-width="7" paint-order="stroke">
      <textPath href="#ribbon-line" startOffset="50%">Sugarcube</textPath>
    </text>`;
}

// --- Variants --------------------------------------------------------------

function defs() {
  return `
    <defs>
      <radialGradient id="age" cx="50%" cy="46%" r="70%">
        <stop offset="55%" stop-color="${C.paperDark}" stop-opacity="0"/>
        <stop offset="100%" stop-color="#B98F55" stop-opacity="0.55"/>
      </radialGradient>
      <filter id="grain" x="0" y="0" width="100%" height="100%">
        <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="4"/>
        <feColorMatrix values="0 0 0 0 0.45  0 0 0 0 0.32  0 0 0 0 0.18  0 0 0 0.55 -0.12"/>
      </filter>
    </defs>`;
}

// The mascot's own drawing box (sparkles to thumb, top of thumb to shoes),
// and where and how big it sits on the box art: the feet tuck behind the ribbon.
const MASCOT = { cx: 549, cy: 494, at: [506, 424], scale: 1.1 };

function placedMascot() {
  const [x, y] = MASCOT.at;
  const k = MASCOT.scale;
  return `<g transform="translate(${x - MASCOT.cx * k},${y - MASCOT.cy * k}) scale(${k})">${mascot()}</g>`;
}

// Android foreground fit, measured from the drawing (see fitAndroid): the
// centre of the art and the scale that puts its farthest point on the
// 66 dp safe circle of the 108 dp layer.
let ANDROID_FIT = { tx: 0, ty: 0, s: 1 };

function svg(variant) {
  let body;
  if (variant === "full") {
    body = paper() + frame() + placedMascot() + banner();
  } else if (variant === "android-bg") {
    body = paper();
  } else if (variant === "android-art" || variant === "mono-art") {
    // Unscaled, for measuring.
    body = `<g id="art">${variant === "mono-art" ? placedMascot() : placedMascot() + banner()}</g>`;
  } else {
    // Android foreground and monochrome: the mascot and ribbon, fitted inside
    // the safe circle so no launcher mask (circle, squircle, teardrop) cuts them.
    const { tx, ty, s } = ANDROID_FIT[variant];
    const content = variant === "mono" ? placedMascot() : placedMascot() + banner();
    body = `<g transform="translate(${tx},${ty}) scale(${s})">${content}</g>`;
    if (variant === "mono") {
      // A mask, so the black cut-outs become holes rather than black ink.
      body = `<mask id="mono" class="mono">${body}</mask><rect width="1024" height="1024" fill="#fff" mask="url(#mono)"/>`;
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">${defs()}${body}</svg>`;
}

// Monochrome (Android 13 themed icons) uses only alpha: line art, with the
// eyes, mouth and shoes solid and the cut-outs punched back out (svg()).
const CSS = `
  @font-face { font-family: Pacifico; src: url(data:font/woff;base64,${fs.readFileSync(FONT).toString("base64")}); }
  html, body { margin: 0; background: transparent; }
  svg { display: block; width: 100vw; height: 100vh; }
  .shape { overflow: hidden; }
  .mono * { stroke: #fff !important; fill: none !important; }
  .mono .solid { fill: #fff !important; }
  .mono .solid-w { fill: none !important; }
  .mono .cut { fill: #000 !important; stroke: none !important; }
  .mono .cheek, .mono .grain { display: none; }
`;

// --- Output ----------------------------------------------------------------

const DENSITIES = { mdpi: 1, hdpi: 1.5, xhdpi: 2, xxhdpi: 3, xxxhdpi: 4 };

async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage();
  const render = async (variant, size, file, { clip = null, transparent = false } = {}) => {
    await page.setViewportSize({ width: size, height: size });
    const radius = clip === "circle" ? "50%" : clip === "rounded" ? "18%" : "0";
    await page.setContent(
      `<style>${CSS} .shape { border-radius: ${radius}; }</style><div class="shape">${svg(variant)}</div>`,
    );
    await page.evaluate(() => document.fonts.ready);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    await page.screenshot({ path: file, omitBackground: transparent || clip !== null });
    console.log(path.relative(REPO, file));
  };

  // iOS: one 1024 opaque icon; the system masks the corners.
  await render("full", 1024, path.join(IOS_SET, "AppIcon.png"));
  fs.writeFileSync(path.join(IOS_SET, "Contents.json"), IOS_CONTENTS);
  fs.writeFileSync(path.join(path.dirname(IOS_SET), "Contents.json"), CATALOG_CONTENTS);

  // Android: adaptive layers (108 dp) plus legacy launchers (48 dp).
  ANDROID_FIT = {
    "android-fg": await fitAndroid(page, "android-art"),
    mono: await fitAndroid(page, "mono-art"),
  };
  for (const [name, k] of Object.entries(DENSITIES)) {
    const dir = path.join(ANDROID_RES, `mipmap-${name}`);
    const layer = Math.round(108 * k);
    const legacy = Math.round(48 * k);
    await render("android-bg", layer, path.join(dir, "ic_launcher_background.png"));
    await render("android-fg", layer, path.join(dir, "ic_launcher_foreground.png"), { transparent: true });
    await render("mono", layer, path.join(dir, "ic_launcher_monochrome.png"), { transparent: true });
    await render("full", legacy, path.join(dir, "ic_launcher.png"), { clip: "rounded" });
    await render("full", legacy, path.join(dir, "ic_launcher_round.png"), { clip: "circle" });
  }
  const anydpi = path.join(ANDROID_RES, "mipmap-anydpi-v26");
  fs.mkdirSync(anydpi, { recursive: true });
  for (const name of ["ic_launcher", "ic_launcher_round"]) {
    fs.writeFileSync(path.join(anydpi, `${name}.xml`), ADAPTIVE);
  }
  await browser.close();
}

// Renders `variant` unscaled and measures its visible pixels: the centre of
// their bounding box, and the farthest of them from it. Returns the transform
// that puts that farthest pixel on the safe circle (33 of 108 dp from centre).
async function fitAndroid(page, variant) {
  const n = 1024;
  await page.setViewportSize({ width: n, height: n });
  await page.setContent(`<style>${CSS}</style>${svg(variant)}`);
  await page.evaluate(() => document.fonts.ready);
  const png = (await page.screenshot({ omitBackground: true })).toString("base64");
  const { cx, cy, r } = await page.evaluate(async ([b64, n]) => {
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = n;
    const ctx = canvas.getContext("2d");
    ctx.drawImage(img, 0, 0);
    const a = ctx.getImageData(0, 0, n, n).data;
    let [x0, y0, x1, y1] = [n, n, 0, 0];
    const pts = [];
    for (let y = 0; y < n; y++) {
      for (let x = 0; x < n; x++) {
        if (a[(y * n + x) * 4 + 3] > 8) {
          x0 = Math.min(x0, x); x1 = Math.max(x1, x);
          y0 = Math.min(y0, y); y1 = Math.max(y1, y);
          pts.push(x, y);
        }
      }
    }
    const [cx, cy] = [(x0 + x1 + 1) / 2, (y0 + y1 + 1) / 2];
    let r = 0;
    for (let i = 0; i < pts.length; i += 2) r = Math.max(r, Math.hypot(pts[i] + 0.5 - cx, pts[i + 1] + 0.5 - cy));
    return { cx, cy, r };
  }, [png, n]);
  const s = (n * 33) / 108 / r;
  return { tx: n / 2 - cx * s, ty: n / 2 - cy * s, s };
}

const IOS_CONTENTS = `{
  "images" : [
    {
      "filename" : "AppIcon.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    }
  ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
`;

const CATALOG_CONTENTS = `{
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
`;

const ADAPTIVE = `<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by packages/mobile/scripts/app_icon.cjs -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
    <monochrome android:drawable="@mipmap/ic_launcher_monochrome" />
</adaptive-icon>
`;

module.exports = { svg, CSS };

if (require.main === module) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
