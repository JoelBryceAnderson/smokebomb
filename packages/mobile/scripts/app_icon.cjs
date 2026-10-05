#!/usr/bin/env node
// Draws the Sugarcube app icon and writes every iOS and Android size.
//
// The icon is an old candy-coloured sugar box: a white panel with a yellow
// sunburst, a pink border and a ribbon with "Sugarcube" in Pacifico (the face the die writes
// its name in). Above it jumps the mascot, a rubber-hose sugar cube with a
// fist in the air.
//
// The art is SVG, rendered by Chromium through Playwright:
//   npm i -g playwright   (or use any install that `require` can find)
//   node packages/mobile/scripts/app_icon.cjs
// He's drawn mid-jump with a fist up; ICON_POSE=stand draws the earlier
// standing thumbs-up instead.

const fs = require("fs");
const path = require("path");
const { chromium } = require("playwright");

const MOBILE = path.resolve(__dirname, "..");
const REPO = path.resolve(MOBILE, "../..");
const FONT = path.join(REPO, "packages/firmware/assets/fonts/pacifico-latin-400-normal.woff");
const IOS_SET = path.join(MOBILE, "iosApp/iosApp/Assets.xcassets/AppIcon.appiconset");
const ANDROID_RES = path.join(MOBILE, "composeApp/src/androidMain/res");

// Candy-box colours: a bubblegum border, a white panel with butter-yellow
// rays, a hot-pink ribbon with white lettering, and raspberry trim.
const C = {
  paper: "#FFFCF5",
  ray: "#FCE6A0",
  age: "#E8B95E",
  border: "#F48FB5",
  ribbon: "#E2487F",
  ribbonDark: "#B02D60",
  trim: "#7C2147",
  lettering: "#FFFFFF",
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

// `border` draws the box's solid border over the ageing (which would
// muddy it) but under the grain, so it shares the paper's texture.
function paper(border = false) {
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
    ${border ? frame() : ""}
    <rect width="1024" height="1024" filter="url(#grain)" opacity="0.35"/>`;
}

// Solid pink from the icon's edge in to a rounded panel, then a pinline
// just inside the panel's edge.
function frame() {
  return `
    <path d="M0,0 H1024 V1024 H0 Z M60,${60 + 176} A176,176 0 0 1 ${60 + 176},60 H${964 - 176} A176,176 0 0 1 964,${60 + 176}
      V${964 - 176} A176,176 0 0 1 ${964 - 176},964 H${60 + 176} A176,176 0 0 1 60,${964 - 176} Z" fill="${C.border}" fill-rule="evenodd"/>
    <rect x="78" y="78" width="868" height="868" rx="158" fill="none" stroke="${C.trim}" stroke-width="6"/>`;
}

// The thumbs-up glove: where it sits in mascot coordinates, its tilt and
// scale, and where its cuff opens (the arm ends there).
const GLOVE = { x: 826, y: 344, tilt: -8, scale: 0.8 };
const CUFF = (() => {
  const [lx, ly] = [-12, 122].map((v) => v * GLOVE.scale);
  const t = (GLOVE.tilt * Math.PI) / 180;
  return [GLOVE.x + lx * Math.cos(t) - ly * Math.sin(t), GLOVE.y + lx * Math.sin(t) + ly * Math.cos(t)];
})();

function glove() {
  // A cartoon glove giving a thumbs up, seen from the front: one outline runs
  // up the back of the hand into the thumb, over its tip, then down the four
  // curled fingers (a bump each, smaller toward the little finger) and round
  // the heel of the palm. Creases run in from between the fingers.
  const { x, y, tilt, scale } = GLOVE;
  const hand = `M-12,-140
    C8,-140 18,-126 16,-108 C14,-92 8,-78 10,-62
    C30,-70 58,-66 66,-46 C74,-30 66,-16 56,-14
    C72,-10 76,10 62,18 C74,26 74,46 58,52
    C66,62 60,78 44,80 C20,88 -24,86 -44,72
    C-64,56 -66,20 -56,-8 C-50,-30 -44,-50 -42,-80
    C-40,-104 -38,-138 -12,-140 Z`;
  const creases = `M56,-14 C40,-14 24,-18 12,-24 M62,18 C44,20 28,16 14,12
    M58,52 C44,54 30,50 20,46 M10,-62 C4,-50 -6,-44 -18,-44`;
  const cuff = "M-48,64 C-58,84 -64,100 -66,112 C-30,128 22,126 44,108 C38,96 30,84 26,76 Z";
  return `
    <g transform="translate(${x},${y}) rotate(${tilt}) scale(${scale})" stroke="${C.ink}" stroke-width="14" stroke-linejoin="round" stroke-linecap="round">
      <path class="solid-w" d="${cuff}" fill="${C.sugarTop}"/>
      <path d="M-62,100 C-28,114 18,112 40,96" fill="none" stroke-width="9"/>
      <path class="solid-w" d="${hand}" fill="${C.sugarTop}"/>
      <path d="${creases}" fill="none" stroke-width="9"/>
      <path d="M-54,-136 L-76,-146 M30,-128 L52,-138" fill="none" stroke-width="11"/>
    </g>`;
}

// The front face's centre line, which the legs and boots mirror about.
const FACE_X = 490;

// "stand" (thumbs up, on the ribbon) or "jump" (mid-air, fist up).
let POSE = process.env.ICON_POSE || "jump";

function standLegs() {
  return `
      <!-- legs and boots, mirrored about the front face's centre -->
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" fill="none">
        <path d="M${FACE_X - 38},632 Q${FACE_X - 44},672 ${FACE_X - 50},706"/>
        <path d="M${FACE_X + 38},632 Q${FACE_X + 44},672 ${FACE_X + 50},706"/>
      </g>
      ${boot(FACE_X - 50, 706, 1)}
      ${boot(FACE_X + 50, 706, -1)}`;
}

function jumpLegs() {
  // Knees tucked: the legs kink out and the boots kick back, toes down,
  // with whoosh lines trailing below.
  return `
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" stroke-linejoin="round" fill="none">
        <path d="M${FACE_X - 38},632 Q${FACE_X - 92},650 ${FACE_X - 74},690"/>
        <path d="M${FACE_X + 38},632 Q${FACE_X + 92},650 ${FACE_X + 74},690"/>
      </g>
      <g transform="rotate(24 ${FACE_X - 74} 690)">${boot(FACE_X - 74, 690, 1)}</g>
      <g transform="rotate(-24 ${FACE_X + 74} 690)">${boot(FACE_X + 74, 690, -1)}</g>
      <g stroke="${C.ink}" stroke-width="9" stroke-linecap="round" fill="none">
        <path d="M${FACE_X - 34},752 Q${FACE_X - 28},772 ${FACE_X - 34},790"/>
        <path d="M${FACE_X},758 L${FACE_X},796"/>
        <path d="M${FACE_X + 34},752 Q${FACE_X + 28},772 ${FACE_X + 34},790"/>
      </g>`;
}

function standArms() {
  return `
      <!-- arm on the hip (left), thumbs-up arm (right) -->
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" fill="none">
        <path d="M334,470 C250,462 238,560 300,582"/>
        <path d="M716,450 C790,452 ${CUFF[0]},${CUFF[1] + 30} ${CUFF[0]},${CUFF[1] - 4}"/>
      </g>
      ${hipGlove(306, 584)}
      ${glove()}`;
}

function jumpArms() {
  // Right arm punches straight up into a fist; the left swings out low.
  return `
      <g stroke="${C.ink}" stroke-width="16" stroke-linecap="round" fill="none">
        <path d="M716,446 C788,438 822,390 830,342"/>
        <path d="M334,480 C270,486 236,520 222,566"/>
      </g>
      ${hipGlove(220, 580)}
      ${fist(836, 278, 14)}
      <path d="M764,250 L744,238 M790,214 L780,198" stroke="${C.ink}" stroke-width="11" stroke-linecap="round" fill="none"/>`;
}

function fist(x, y, tilt = 0) {
  // A raised cartoon fist seen from the front, wrist at the bottom: four
  // knuckle bumps across the top, the thumb folded across the fingers, and
  // a flared cuff.
  const hand = `M-50,26 C-62,0 -58,-38 -40,-50 C-32,-64 -14,-66 -8,-54
    C0,-68 18,-68 22,-54 C30,-66 48,-64 50,-48 C62,-44 64,-20 56,-6
    C62,14 56,32 42,40 C20,50 -32,48 -50,26 Z`;
  const thumb = "M-52,-4 C-32,-18 8,-20 28,-8 C36,-2 32,12 22,12 C2,10 -30,12 -48,20 Z";
  const cuff = "M-36,38 C-44,54 -48,64 -50,76 C-20,88 24,88 50,74 C46,62 40,50 36,38 Z";
  return `
    <g transform="translate(${x},${y}) rotate(${tilt}) scale(0.82)" stroke="${C.ink}" stroke-width="14" stroke-linejoin="round" stroke-linecap="round">
      <path class="solid-w" d="${cuff}" fill="${C.sugarTop}"/>
      <path d="M-46,64 C-14,76 22,76 46,62" fill="none" stroke-width="9"/>
      <path class="solid-w" d="${hand}" fill="${C.sugarTop}"/>
      <path d="M-8,-54 C-10,-46 -10,-38 -8,-30 M22,-54 C22,-46 22,-38 23,-30 M50,-48 C48,-40 48,-32 49,-26" fill="none" stroke-width="7"/>
      <path class="solid-w" d="${thumb}" fill="${C.sugarTop}"/>
    </g>`;
}

function boot(x, y, side) {
  // A chubby cartoon boot with its ankle at (x, y): a round toe turned
  // outward (side 1 is the left foot), a pink cuff and a shine on the toe.
  return `
    <g transform="translate(${x},${y}) scale(${side},1)" stroke="${C.ink}" stroke-width="10" stroke-linejoin="round">
      <path class="solid" d="M-18,-6 L18,-6 C20,10 24,22 30,30 C36,40 30,50 16,50 L-46,50
        C-70,50 -76,26 -58,16 C-46,8 -26,10 -20,4 Z" fill="${C.ink}"/>
      <ellipse class="cut" cx="-48" cy="26" rx="12" ry="6" fill="#FFFFFF" opacity="0.55" stroke="none" transform="rotate(-18 -48 26)"/>
      <rect class="solid-w" x="-27" y="-22" width="54" height="26" rx="13" fill="${C.border}" stroke-width="8"/>
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
  const tilt = POSE === "jump" ? `rotate(-5 ${FACE_X} 480)` : "";
  return `
    <g class="mascot" transform="${tilt}">
      ${POSE === "jump" ? jumpLegs() : standLegs()}
      <!-- the cube -->
      <path class="solid-w" d="${side}" fill="${C.sugarSide}"/>
      <path class="solid-w" d="${top}" fill="${C.sugarTop}"/>
      <path class="solid-w" d="${front}" fill="${C.sugar}"/>
      <path d="${edges}" fill="none" stroke="${C.ink}" stroke-width="10" stroke-linecap="round" stroke-linejoin="round"/>
      <path d="${outline}" fill="none" stroke="${C.ink}" stroke-width="14" stroke-linejoin="round"/>
      <g class="grain" fill="${C.sugarSide}">${grains.join("")}</g>
      ${POSE === "jump" ? jumpArms() : standArms()}
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

// The ribbon: a band bent along a circular arc, so it keeps its depth all the
// way to the ends where the lettering tilts. The arc's chord runs from x0 to
// x1 at height y (the band's centre line) and rises by `lift` in the middle;
// the band is `h` deep, with notched tails behind each end.
const RIBBON = { x0: 176, x1: 848, y: 850, lift: 70, h: 184 };
// Pacifico's ink for "Sugarcube", in em: the b's ascender to the g's descender.
const WORD_INK = { top: 0.94, bottom: -0.455 };
// The lettering fills this share of the band's depth.
const WORD_FILL = 0.74;

function banner() {
  const { x0, x1, y, lift, h } = RIBBON;
  const half = (x1 - x0) / 2;
  const R = (half * half + lift * lift) / (2 * lift);
  const [cx, cy] = [(x0 + x1) / 2, y - lift + R];
  const theta = Math.asin(half / R);
  const deg = (theta * 180) / Math.PI;
  const at = (r, a) => [cx + r * Math.sin(a), cy - r * Math.cos(a)];
  const arc = (r, from, to) => {
    const [a, b] = [at(r, from), at(r, to)];
    const sweep = to > from ? 1 : 0;
    return `${a[0].toFixed(1)},${a[1].toFixed(1)} A${r.toFixed(1)},${r.toFixed(1)} 0 0 ${sweep} ${b[0].toFixed(1)},${b[1].toFixed(1)}`;
  };
  const [ro, ri] = [R + h / 2, R - h / 2];
  const band = `M${arc(ro, -theta, theta)} L${arc(ri, theta, -theta)} Z`;
  // A tail drawn for the left end with the band running along +x and ending
  // at x = 0, then turned to the band's slope there; mirrored for the right.
  const v = h / 2;
  const tailShape = `
    <path class="solid" d="M34,${-v + 40} L-78,${-v + 40} L-46,4 L-78,${v + 6} L34,${v + 14} Z" fill="${C.ribbonDark}"/>
    <path class="solid" d="M0,${v} L34,${v + 14} L34,${v - 6} Z" fill="${C.trim}"/>`;
  const [lx, ly] = at(R, -theta);
  const fontSize = (WORD_FILL * h) / (WORD_INK.top - WORD_INK.bottom);
  // Shift the baseline down so the ink is centred on the band's centre line.
  const dy = ((WORD_INK.top + WORD_INK.bottom) / 2) * fontSize;
  return `
    <defs><path id="ribbon-line" d="M${arc(R, -theta, theta)}"/></defs>
    <g class="banner" stroke="${C.trim}" stroke-width="10" stroke-linejoin="round">
      <g transform="translate(${lx},${ly}) rotate(${-deg})">${tailShape}</g>
      <g transform="translate(${2 * cx - lx},${ly}) rotate(${deg}) scale(-1,1)">${tailShape}</g>
      <path class="solid" d="${band}" fill="${C.ribbon}"/>
      <path d="M${arc(ro - 16, -theta, theta)} M${arc(ri + 16, -theta, theta)}" fill="none" stroke="${C.lettering}" stroke-width="4" opacity="0.7"/>
    </g>
    <text class="word" font-family="Pacifico" font-size="${fontSize.toFixed(1)}" dy="${dy.toFixed(1)}" text-anchor="middle"
      fill="${C.lettering}" stroke="${C.trim}" stroke-width="7" paint-order="stroke">
      <textPath href="#ribbon-line" startOffset="50%">Sugarcube</textPath>
    </text>`;
}

// --- Variants --------------------------------------------------------------

function defs() {
  return `
    <defs>
      <radialGradient id="age" cx="50%" cy="46%" r="70%">
        <stop offset="60%" stop-color="${C.age}" stop-opacity="0"/>
        <stop offset="100%" stop-color="${C.age}" stop-opacity="0.3"/>
      </radialGradient>
      <filter id="grain" x="0" y="0" width="100%" height="100%">
        <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="4"/>
        <feColorMatrix values="0 0 0 0 0.45  0 0 0 0 0.32  0 0 0 0 0.18  0 0 0 0.55 -0.12"/>
      </filter>
    </defs>`;
}

// The cube's horizontal centre in mascot coordinates (its silhouette runs
// from 330 to 718). He's centred on the cube, not on the whole drawing, so the
// raised arm off to one side doesn't pull the cube off centre.
const CUBE_X = 524;
// The mascot's vertical centre in its own coordinates, and where (y) and how
// big it sits on the box art. Standing, the boots rest on the ribbon; jumping,
// he's smaller and higher, so the fist clears the border and there's air
// between his boots and the ribbon.
const MASCOT = { cy: 494, stand: { y: 404, scale: 1.08 }, jump: { y: 388, scale: 0.95 } };

function placedMascot() {
  const { y, scale: k } = MASCOT[POSE];
  return `<g transform="translate(${512 - CUBE_X * k},${y - MASCOT.cy * k}) scale(${k})">${mascot()}</g>`;
}

// Android foreground fit, measured from the drawing (see fitAndroid): the
// centre of the art and the scale that puts its farthest point on the
// 66 dp safe circle of the 108 dp layer.
let ANDROID_FIT = { tx: 0, ty: 0, s: 1 };

function svg(variant) {
  let body;
  if (variant === "full") {
    body = paper(true) + placedMascot() + banner();
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

// Renders `variant` unscaled and measures its visible pixels. The art is
// already centred horizontally on the cube (placedMascot), so only the
// vertical centre comes from their bounding box. Returns the transform that
// keeps that centre and puts the farthest pixel from it on the safe circle
// (33 of 108 dp from centre).
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
    const [cx, cy] = [n / 2, (y0 + y1 + 1) / 2];
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

module.exports = { svg, CSS, setPose: (p) => (POSE = p) };

if (require.main === module) {
  main().catch((e) => {
    console.error(e);
    process.exit(1);
  });
}
