// GBC on a cube: the browser side.
//
// The page owns the cube's pose (resting face, tilt, rolls) and sends what
// the die's sensors would read: the accelerometer in die axes and which
// faces are touched. The Rust server runs the emulator and the cube logic
// and streams back the six faces, the emulator's frame and status.

import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
import "./style.css";

// Each face's drawing axes in die coordinates: canvas right, canvas up and
// the outward normal (smokebomb_core::orientation::BASES), in Face order.
type V3 = [number, number, number];
const BASES: { x: V3; y: V3; n: V3 }[] = [
  { x: [0, 0, -1], y: [0, 1, 0], n: [1, 0, 0] },
  { x: [0, 0, 1], y: [0, 1, 0], n: [-1, 0, 0] },
  { x: [1, 0, 0], y: [0, 0, -1], n: [0, 1, 0] },
  { x: [1, 0, 0], y: [0, 0, 1], n: [0, -1, 0] },
  { x: [1, 0, 0], y: [0, 1, 0], n: [0, 0, 1] },
  { x: [-1, 0, 0], y: [0, 1, 0], n: [0, 0, -1] },
];
const NAMES = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"];
const FACE = 64;
const LCD_W = 160;
const LCD_H = 144;
const HALF = 0.5;
// The 30 mm die's panels light 10.75 mm of each 30 mm face; drawn bigger
// here so the pixels can be seen.
const PANEL = 0.88;

// ---------------------------------------------------------------- scene
const canvas = document.getElementById("three") as HTMLCanvasElement;
const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, preserveDrawingBuffer: true });
renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
const scene = new THREE.Scene();
scene.background = new THREE.Color(0x16161a);
const camera = new THREE.PerspectiveCamera(32, 1, 0.1, 100);
camera.position.set(0, 3.1, 3.9);
const controls = new OrbitControls(camera, canvas);
controls.target.set(0, 0.45, 0);
controls.enableDamping = true;
controls.update();

scene.add(new THREE.HemisphereLight(0xffffff, 0x303038, 1.6));
const sun = new THREE.DirectionalLight(0xffffff, 1.2);
sun.position.set(2, 4, 3);
scene.add(sun);
const table = new THREE.Mesh(
  new THREE.CircleGeometry(4, 64),
  new THREE.MeshStandardMaterial({ color: 0x2a2a31, roughness: 0.9 }),
);
table.rotation.x = -Math.PI / 2;
scene.add(table);
// A compass rose on the table: which way map north was when the page
// loaded (it follows the world, not the die).
const rose = new THREE.Mesh(
  new THREE.RingGeometry(1.15, 1.18, 64),
  new THREE.MeshBasicMaterial({ color: 0x3a3a44 }),
);
rose.rotation.x = -Math.PI / 2;
rose.position.y = 0.001;
scene.add(rose);
const northMark = new THREE.Mesh(
  new THREE.ConeGeometry(0.06, 0.16, 3),
  new THREE.MeshBasicMaterial({ color: 0xf5c451 }),
);
northMark.rotation.x = -Math.PI / 2;
northMark.position.set(0, 0.002, -1.25);
scene.add(northMark);

const die = new THREE.Group();
scene.add(die);
die.add(
  new THREE.Mesh(
    new RoundedBoxGeometry(1, 1, 1, 4, 0.06),
    new THREE.MeshStandardMaterial({ color: 0xf4f1ea, roughness: 0.5 }),
  ),
);

interface FaceView {
  mesh: THREE.Mesh;
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
  image: ImageData;
  texture: THREE.CanvasTexture;
  thumb: CanvasRenderingContext2D;
}
const thumbs = document.getElementById("faces")!;
const faces: FaceView[] = BASES.map((b, i) => {
  const c = document.createElement("canvas");
  c.width = c.height = FACE;
  const ctx = c.getContext("2d")!;
  const texture = new THREE.CanvasTexture(c);
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.colorSpace = THREE.SRGBColorSpace;
  const mesh = new THREE.Mesh(
    new THREE.PlaneGeometry(PANEL, PANEL),
    new THREE.MeshBasicMaterial({ map: texture, toneMapped: false }),
  );
  // Plane x/y/z = the face's drawing x, y and normal.
  const m = new THREE.Matrix4().makeBasis(
    new THREE.Vector3(...b.x),
    new THREE.Vector3(...b.y),
    new THREE.Vector3(...b.n),
  );
  mesh.quaternion.setFromRotationMatrix(m);
  mesh.position.set(...b.n).multiplyScalar(HALF + 0.003);
  mesh.userData.face = i;
  die.add(mesh);
  const cell = document.createElement("div");
  const t = document.createElement("canvas");
  t.width = t.height = FACE;
  cell.append(t, NAMES[i]);
  thumbs.append(cell);
  return { mesh, canvas: c, ctx, image: ctx.createImageData(FACE, FACE), texture, thumb: t.getContext("2d")! };
});

function resize() {
  const r = canvas.parentElement!.getBoundingClientRect();
  renderer.setSize(r.width, r.height, false);
  camera.aspect = r.width / r.height;
  camera.updateProjectionMatrix();
}
window.addEventListener("resize", resize);
resize();

// ---------------------------------------------------------------- pose
// World: +Y up, map north along -Z (away from the default camera), east +X.
const UP = new THREE.Vector3(0, 1, 0);
const DIRS: Record<string, THREE.Vector3> = {
  n: new THREE.Vector3(0, 0, -1),
  s: new THREE.Vector3(0, 0, 1),
  e: new THREE.Vector3(1, 0, 0),
  w: new THREE.Vector3(-1, 0, 0),
};
let base = new THREE.Quaternion(); // die → world, resting on a face
const tilt = new THREE.Vector2(); // current lean: x east, y south (radians)
const tiltTarget = new THREE.Vector2();
let roll: { dir: THREE.Vector3; start: number; from: number } | null = null;
const ROLL_MS = 420;
const TILT_RAD = (24 * Math.PI) / 180;
let shakeUntil = 0;

/** The die's pose for a lean of `angle` toward horizontal `dir`: it pivots
 * on the bottom edge on that side, like a real cube tipped by hand. */
function pose(dir: THREE.Vector3, angle: number) {
  const q = new THREE.Quaternion();
  const pos = new THREE.Vector3(0, HALF, 0);
  if (angle > 1e-6) {
    const axis = new THREE.Vector3().crossVectors(UP, dir).normalize();
    q.setFromAxisAngle(axis, angle);
    const pivot = new THREE.Vector3(dir.x * HALF, 0, dir.z * HALF);
    pos.sub(pivot).applyQuaternion(q).add(pivot);
  }
  return { q: q.multiply(base), pos };
}

function startRoll(d: string) {
  if (roll) return;
  const dir = DIRS[d];
  const lean = tilt.length();
  roll = { dir, start: performance.now(), from: lean > 0 ? lean : 0 };
}

function updatePose(now: number) {
  if (roll) {
    const t = Math.min(1, (now - roll.start) / ROLL_MS);
    const e = t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2;
    const a = roll.from + (Math.PI / 2 - roll.from) * e;
    const p = pose(roll.dir, a);
    die.quaternion.copy(p.q);
    die.position.copy(p.pos);
    if (t >= 1) {
      // Settle on the new face, snapped to exact quarter turns.
      const axis = new THREE.Vector3().crossVectors(UP, roll.dir).normalize();
      base = new THREE.Quaternion().setFromAxisAngle(axis, Math.PI / 2).multiply(base).normalize();
      snap(base);
      tilt.set(0, 0);
      roll = null;
    }
    return;
  }
  tilt.lerp(tiltTarget, 0.18);
  const v = new THREE.Vector3(tilt.x, 0, tilt.y);
  const a = v.length();
  const p = pose(a > 0 ? v.normalize() : DIRS.n, a);
  die.quaternion.copy(p.q);
  die.position.copy(p.pos);
}

function snap(q: THREE.Quaternion) {
  const m = new THREE.Matrix4().makeRotationFromQuaternion(q);
  const e = m.elements;
  for (let i = 0; i < 16; i++) e[i] = Math.round(e[i]);
  q.setFromRotationMatrix(m);
}

/** What the accelerometer reads: the sky in die axes, milli-g. */
function accel(now: number): V3 {
  const sky = UP.clone().applyQuaternion(die.quaternion.clone().invert()).multiplyScalar(1000);
  if (now < shakeUntil) {
    const s = Math.sin(now / 22) * 2600;
    sky.x += s;
    sky.z += Math.cos(now / 17) * 900;
  }
  return [sky.x, sky.y, sky.z];
}

// ---------------------------------------------------------------- input
let keys = 0;
let touch = 0;
const KEYPAD: Record<string, number> = {
  KeyZ: 0x01, KeyX: 0x02, Backspace: 0x04, Enter: 0x08,
  ArrowRight: 0x10, ArrowLeft: 0x20, ArrowUp: 0x40, ArrowDown: 0x80,
};
const TILT_KEYS: Record<string, string> = { KeyW: "n", KeyS: "s", KeyA: "w", KeyD: "e" };
const tiltHeld = new Set<string>();

function applyTiltKeys() {
  const v = new THREE.Vector3();
  for (const d of tiltHeld) v.add(DIRS[d]);
  if (v.lengthSq() > 0) v.normalize().multiplyScalar(TILT_RAD);
  tiltTarget.set(v.x, v.z);
}

window.addEventListener("keydown", (e) => {
  if ((e.target as HTMLElement).tagName === "INPUT" && e.code !== "Escape") return;
  if (e.code in TILT_KEYS) {
    if (e.shiftKey) startRoll(TILT_KEYS[e.code]);
    else tiltHeld.add(TILT_KEYS[e.code]);
    applyTiltKeys();
  } else if (e.code in KEYPAD) {
    keys |= KEYPAD[e.code];
    send({ type: "keys", buttons: keys });
  } else if (e.code === "Space") {
    shakeUntil = performance.now() + 700;
  } else return;
  e.preventDefault();
});
window.addEventListener("keyup", (e) => {
  if (e.code in TILT_KEYS) {
    tiltHeld.delete(TILT_KEYS[e.code]);
    applyTiltKeys();
  } else if (e.code in KEYPAD) {
    keys &= ~KEYPAD[e.code];
    send({ type: "keys", buttons: keys });
  }
});

// Clicking a face touches it (hold for a long press); dragging elsewhere
// orbits the view.
const ray = new THREE.Raycaster();
let touchedFace = -1;
canvas.addEventListener("pointerdown", (e) => {
  const r = canvas.getBoundingClientRect();
  ray.setFromCamera(
    new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1),
    camera,
  );
  const hit = ray.intersectObjects(faces.map((f) => f.mesh))[0];
  if (!hit) return;
  touchedFace = hit.object.userData.face;
  touch |= 1 << touchedFace;
  controls.enabled = false;
  send({ type: "touch", mask: touch });
});
window.addEventListener("pointerup", () => {
  if (touchedFace < 0) return;
  touch &= ~(1 << touchedFace);
  touchedFace = -1;
  controls.enabled = true;
  send({ type: "touch", mask: touch });
});

// The tilt pad: drag the knob to lean the cube.
const pad = document.getElementById("pad")!;
const knob = document.getElementById("knob")!;
let padActive = false;
function padMove(e: PointerEvent) {
  const r = pad.getBoundingClientRect();
  let x = (e.clientX - r.left) / r.width - 0.5;
  let y = (e.clientY - r.top) / r.height - 0.5;
  const l = Math.hypot(x, y);
  if (l > 0.5) {
    x *= 0.5 / l;
    y *= 0.5 / l;
  }
  knob.style.left = `${(x + 0.5) * r.width - 11}px`;
  knob.style.top = `${(y + 0.5) * r.height - 11}px`;
  // Full deflection: 60°, enough to roll.
  const k = (Math.PI / 3) / 0.5;
  tiltTarget.set(x * k, y * k);
}
pad.addEventListener("pointerdown", (e) => {
  padActive = true;
  pad.setPointerCapture(e.pointerId);
  padMove(e);
});
pad.addEventListener("pointermove", (e) => padActive && padMove(e));
pad.addEventListener("pointerup", () => {
  padActive = false;
  knob.style.left = knob.style.top = "37px";
  applyTiltKeys();
});

for (const b of document.querySelectorAll<HTMLButtonElement>("[data-roll]")) {
  b.addEventListener("click", () => startRoll(b.dataset.roll!));
}
document.getElementById("shake")!.addEventListener("click", () => (shakeUntil = performance.now() + 700));
document.getElementById("reset")!.addEventListener("click", () => send({ type: "reset" }));

const settings = [
  "wrap", "ui", "fog", "deadzone", "walk_max_deg", "walk_hold_ms", "switch_deg", "long_press_ms", "debug",
] as const;
function sendConfig() {
  const msg: Record<string, unknown> = { type: "config" };
  for (const id of settings) {
    const el = document.getElementById(id) as HTMLInputElement | HTMLSelectElement;
    if (el instanceof HTMLInputElement && el.type === "checkbox") msg[id] = el.checked;
    else if (el instanceof HTMLInputElement) msg[id] = Number(el.value);
    else msg[id] = el.value;
  }
  document.getElementById("predfig")!.hidden = !(msg.debug as boolean);
  send(msg);
}
for (const id of settings) document.getElementById(id)!.addEventListener("input", sendConfig);

// ---------------------------------------------------------------- network
let ws: WebSocket | null = null;
function send(msg: unknown) {
  if (ws?.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}
const lcd = (document.getElementById("lcd") as HTMLCanvasElement).getContext("2d")!;
const pred = (document.getElementById("pred") as HTMLCanvasElement).getContext("2d")!;
const lcdImage = lcd.createImageData(LCD_W, LCD_H);
const predImage = pred.createImageData(LCD_W, LCD_H);
const statusEl = document.getElementById("status")!;
let frames = 0;

function rgb565(dst: Uint8ClampedArray, i: number, c: number) {
  const r = c >> 11, g = (c >> 5) & 0x3f, b = c & 0x1f;
  dst[i] = (r << 3) | (r >> 2);
  dst[i + 1] = (g << 2) | (g >> 4);
  dst[i + 2] = (b << 3) | (b >> 2);
  dst[i + 3] = 255;
}

function onFrame(buf: ArrayBuffer) {
  const head = new Uint8Array(buf, 0, 12);
  if (head[0] !== 0x47 || head[3] !== 0x31) return; // "GCB1"
  const hasPrediction = (head[8] & 1) !== 0;
  const px = new Uint16Array(buf, 12);
  for (let f = 0; f < 6; f++) {
    const v = faces[f];
    const d = v.image.data;
    for (let i = 0; i < FACE * FACE; i++) rgb565(d, i * 4, px[f * FACE * FACE + i]);
    v.ctx.putImageData(v.image, 0, 0);
    v.thumb.putImageData(v.image, 0, 0);
    v.texture.needsUpdate = true;
  }
  const off = 6 * FACE * FACE;
  for (let i = 0; i < LCD_W * LCD_H; i++) rgb565(lcdImage.data, i * 4, px[off + i]);
  lcd.putImageData(lcdImage, 0, 0);
  if (hasPrediction) {
    const p = off + LCD_W * LCD_H;
    let bad = 0;
    for (let i = 0; i < LCD_W * LCD_H; i++) {
      const c = px[p + i];
      rgb565(predImage.data, i * 4, c);
      if (c !== px[off + i]) {
        bad++;
        predImage.data.set([255, 0, 255, 255], i * 4); // mismatches in magenta
      }
    }
    pred.putImageData(predImage, 0, 0);
    document.getElementById("mismatch")!.textContent = `${bad} px differ`;
  }
  frames++;
}

function onStatus(s: Record<string, unknown>) {
  if (s.title) document.getElementById("title")!.textContent = `${s.title}${s.demo ? " (demo cart)" : ""}`;
  const btn = Number(s.buttons ?? 0);
  const names = ["A", "B", "Sel", "Start", "→", "←", "↑", "↓"].filter((_, i) => btn & (1 << i));
  const lines = [
    `scene ${s.scene}   drawn ${s.drawn}${s.reused ? " (kept)" : ""}`,
    `up ${s.up}   N ${s.north}  E ${s.east}  S ${s.south}  W ${s.west}   rolls ${s.rolls}`,
    `walking ${s.walking ?? "-"}   buttons ${names.join(" ") || "-"}`,
    `camera ${JSON.stringify(s.camera)}  player at ${JSON.stringify(s.centre)} on screen`,
    `ui tiles ${s.ui_tiles}  stray ${s.stray_tiles}   frame ${s.frame}`,
    `fps ${Number(s.fps ?? 0).toFixed(1)}   emulator ${s.emu_us ?? "-"} µs   cube ${s.render_us ?? "-"} µs`,
    `ROM bank switches/frame ${s.bank_changes_per_frame ?? "-"}   banks/s ${s.banks_touched ?? "-"}`,
    `renderer vs next frame: ${s.mismatches ?? "-"} px differ`,
  ];
  statusEl.textContent = lines.join("\n");
  lastStatus = s;
}
let lastStatus: Record<string, unknown> = {};

function connect() {
  const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;
  ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";
  ws.onopen = () => sendConfig();
  ws.onmessage = (m) => {
    if (typeof m.data === "string") onStatus(JSON.parse(m.data));
    else {
      onFrame(m.data);
      // Pull the next frame only once this one is drawn.
      ws?.send('{"type":"ready"}');
    }
  };
  ws.onclose = () => {
    statusEl.textContent = "disconnected: is `gbc-cube serve` running? retrying...";
    setTimeout(connect, 1000);
  };
}
connect();

// ---------------------------------------------------------------- loop
let lastImu = 0;
function loop(now: number) {
  updatePose(now);
  if (now - lastImu > 15) {
    lastImu = now;
    send({ type: "imu", accel: accel(now) });
  }
  controls.update();
  renderer.render(scene, camera);
  requestAnimationFrame(loop);
}
requestAnimationFrame(loop);

// For scripted demos and screenshots (see tools/record.mjs).
declare global {
  interface Window {
    gbc: Record<string, unknown>;
  }
}
window.gbc = {
  roll: (d: string) => startRoll(d),
  tilt: (east: number, south: number) => tiltTarget.set(east, south),
  keys: (bits: number) => {
    keys = bits;
    send({ type: "keys", buttons: keys });
  },
  touch: (mask: number) => {
    touch = mask;
    send({ type: "touch", mask });
  },
  shake: () => (shakeUntil = performance.now() + 700),
  config: (c: Record<string, unknown>) => send({ type: "config", ...c }),
  view: (x: number, y: number, z: number) => {
    camera.position.set(x, y, z);
    controls.update();
  },
  status: () => lastStatus,
  frames: () => frames,
  rolling: () => roll !== null,
};
