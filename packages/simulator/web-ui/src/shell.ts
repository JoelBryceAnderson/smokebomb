// Charging contacts and the charging-face etching (design brief §1 and §2).
import * as THREE from "three";
import { DieGeometry, MM } from "./geometry";

// 34 mm die: contact centres 13.2 mm from the face centre on both axes,
// which keeps them clear of the panel's ledge glass (do not move inward).
const HEX_R = 0.81 * MM; // 1.4 mm across flats
const HEX_GAP = 0.15 * MM;

export interface FaceDef {
  n: [number, number, number];
  rot: [number, number, number];
}

/**
 * The four charging contacts on the charging face (the lid); the five cup
 * faces are plain metal. Each contact is a separate plated head, solid and
 * one-piece, so the slot across it is only cosmetic. Under it sits the dark
 * hairline gap ring; all slots point at the window centre and a hex flat
 * faces it too. + is on two diagonal corners and ground on the other two, an
 * active bridge rectifier takes either polarity, so every rotation charges.
 */
export function addContacts(
  die: THREE.Group,
  face: FaceDef,
  half: number,
  g: DieGeometry,
  darkMaterial: THREE.Material,
  contactMaterial: THREE.Material,
  anisotropy: number,
) {
  if (g.screws.kind === "slotted") {
    addSlottedScrews(die, face, half, g.screws, contactMaterial, anisotropy);
    return;
  }
  const CONTACT_AT = g.screws.atMm * MM;
  const ringGeo = new THREE.RingGeometry(HEX_R, HEX_R + HEX_GAP, 6, 1);
  const headGeo = new THREE.CircleGeometry(HEX_R, 6);
  const slotGeo = new THREE.PlaneGeometry(1.3 * MM, 0.2 * MM);
  for (const [sx, sy] of [
    [1, 1],
    [1, -1],
    [-1, 1],
    [-1, -1],
  ]) {
    const g = new THREE.Group();
    const flat = Math.atan2(-sy, -sx) - Math.PI / 6;
    const head = new THREE.Mesh(headGeo, contactMaterial);
    head.rotation.z = flat;
    const ring = new THREE.Mesh(ringGeo, darkMaterial);
    ring.rotation.z = flat;
    ring.position.z = 0.0001;
    const slot = new THREE.Mesh(slotGeo, darkMaterial);
    slot.rotation.z = Math.atan2(-sy, -sx);
    slot.position.z = 0.0002;
    g.add(head, ring, slot);
    g.rotation.set(...face.rot);
    // ~0.01 mm above the metal (0.00075 u) to stay clear of z-fighting.
    g.position.set(...face.n).multiplyScalar(half + 0.0009);
    g.translateX(sx * CONTACT_AT);
    g.translateY(sy * CONTACT_AT);
    die.add(g);
  }
}

/**
 * The 30 mm die's lid screws, which are also its charging contacts: plain
 * round single-slot heads (a catalogue M1.0 watch screw, 1.6 mm across) in a
 * round counterbore whose dark ring is the insulating sleeve, with a 0.25 mm
 * slot. Not clocked: each screw stops where its torque left it, so the slots
 * point every which way (the mockup's angles). Head, slot and ring are one
 * mipmapped texture so the fine slot filters cleanly when the die is small.
 */
function addSlottedScrews(
  die: THREE.Group,
  face: FaceDef,
  half: number,
  s: { atMm: number; headMm: number; ringMm: number; slotMm: number; slotAngles: number[] },
  material: THREE.MeshStandardMaterial | THREE.Material,
  anisotropy: number,
) {
  const N = 256;
  const c = document.createElement("canvas");
  c.width = c.height = N;
  const x = c.getContext("2d")!;
  const px = N / 2 / s.ringMm; // canvas px per mm
  x.fillStyle = "#121214";
  x.fillRect(0, 0, N, N); // the sleeve ring
  x.fillStyle = "#ffffff";
  x.beginPath();
  x.arc(N / 2, N / 2, (s.headMm / 2) * px, 0, Math.PI * 2);
  x.fill();
  x.fillStyle = "#121214";
  x.fillRect(N / 2 - (s.headMm / 2) * px, N / 2 - (s.slotMm / 2) * px, s.headMm * px, s.slotMm * px);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  tex.anisotropy = anisotropy;
  // The head takes the contact finish; the texture darkens the ring and slot.
  const mat = (material as THREE.MeshStandardMaterial).clone();
  mat.map = tex;
  mat.polygonOffset = true;
  mat.polygonOffsetFactor = -2;
  mat.polygonOffsetUnits = -2;
  const geo = new THREE.CircleGeometry(s.ringMm * MM, 64);
  const corners = [
    [1, 1],
    [1, -1],
    [-1, 1],
    [-1, -1],
  ];
  corners.forEach(([sx, sy], i) => {
    const head = new THREE.Mesh(geo, mat);
    head.rotation.z = s.slotAngles[i % s.slotAngles.length];
    const grp = new THREE.Group();
    grp.add(head);
    grp.rotation.set(...face.rot);
    grp.position.set(...face.n).multiplyScalar(half + 0.0009);
    grp.translateX(sx * s.atMm * MM);
    grp.translateY(sy * s.atMm * MM);
    die.add(grp);
  });
}

/**
 * The seam where the lid (the charging face plus the lower half of its four
 * edges) meets the five-sided cup: a hairline in the plane that cuts the
 * bottom edges halfway round their radius, following the die's own curves.
 */
export function addSeam(die: THREE.Group, half: number, edgeRadius: number, material: THREE.Material) {
  const ec = half - edgeRadius;
  const a = ec + edgeRadius / Math.SQRT2;
  const r = edgeRadius / Math.SQRT2;
  const pts: THREE.Vector3[] = [];
  const corners: [number, number, number][] = [
    [ec, ec, 0],
    [-ec, ec, Math.PI / 2],
    [-ec, -ec, Math.PI],
    [ec, -ec, Math.PI * 1.5],
  ];
  for (const [cx, cz, a0] of corners) {
    for (let k = 0; k <= 24; k++) {
      const t = a0 + ((Math.PI / 2) * k) / 24;
      pts.push(new THREE.Vector3(cx + r * Math.cos(t), -a, cz + r * Math.sin(t)));
    }
  }
  const path = new THREE.CatmullRomCurve3(pts, true, "centripetal");
  die.add(new THREE.Mesh(new THREE.TubeGeometry(path, 400, 0.06 * MM, 6, true), material));
}

/** The face that goes down in the Nest: index 3, −Y. */
export const CHARGING_FACE = 3;

const ETCH_TEXT = {
  /** The wordmark, set in the brand's retro script. */
  top: "Sugarcube",
  /** The tagline, in the same script. */
  bottom: "Designed in Williamsburg, BK · Shake well before serving",
  left: "REGULATORY INFO IN SETTINGS",
};

/** Script sizes, and the script ink's clearance from the window. */
const WORDMARK_MM = 0.95;
const TAGLINE_MM = 0.7;
const SCRIPT_CLEAR_MM = 0.8;

function drawEtching(c: HTMLCanvasElement, serial: string, g: DieGeometry) {
  const C = c.width;
  // The canvas covers the flat face: 29 mm on the 34 mm die, 25 on the 30.
  const PXMM = C / g.etch.flatMm;
  const WINDOW_HALF_MM = g.etch.windowHalfMm;
  const x = c.getContext("2d")!;
  x.clearRect(0, 0, C, C);
  x.fillStyle = x.strokeStyle = "rgb(150,152,156)";
  x.textAlign = "center";
  x.textBaseline = "middle";
  const band = C / 2 - g.etch.bandMm * PXMM; // centre line of the border
  const font = (mm: number) => `600 ${mm * PXMM}px "Space Grotesk", Arial, sans-serif`;
  const side = (rot: number, text: string, mm: number) => {
    x.save();
    x.translate(C / 2, C / 2);
    x.rotate(rot);
    x.font = font(mm);
    x.fillText(text, 0, -(C / 2 - band));
    x.restore();
  };
  // Script lines. Their tails hang well below the caps' box, so place them
  // by their ink, with the lowest tail SCRIPT_CLEAR_MM off the window.
  const scriptSide = (rot: number, text: string, mm: number) => {
    x.save();
    x.translate(C / 2, C / 2);
    x.rotate(rot);
    x.font = `400 ${mm * PXMM}px "Pacifico", cursive`;
    const ink = x.measureText(text);
    // 34 mm: hug the window, the lowest tail SCRIPT_CLEAR_MM off it. 30 mm:
    // the border is wide, so centre the ink in it.
    const y = g.etch.centreScripts
      ? -(C / 2 - band) + (ink.actualBoundingBoxAscent - ink.actualBoundingBoxDescent) / 2
      : -(WINDOW_HALF_MM + SCRIPT_CLEAR_MM) * PXMM - ink.actualBoundingBoxDescent;
    x.fillText(text, 0, y);
    x.restore();
  };
  scriptSide(0, ETCH_TEXT.top, WORDMARK_MM);
  side(Math.PI / 2, `SC-1  ·  S/N ${serial}`, 0.7);
  scriptSide(Math.PI, ETCH_TEXT.bottom, TAGLINE_MM);
  // Left: CE, a crossed-out wheelie bin, and where the rest lives.
  x.save();
  x.translate(C / 2, C / 2);
  x.rotate(-Math.PI / 2);
  x.translate(0, -(C / 2 - band));
  x.font = font(0.62);
  x.fillText("CE      " + ETCH_TEXT.left, 0, 0);
  const bx = -5.2 * PXMM;
  const s = 0.36 * PXMM;
  x.lineWidth = 0.07 * PXMM;
  x.strokeRect(bx - s * 0.6, -s * 0.8, s * 1.2, s * 1.6);
  x.beginPath();
  x.moveTo(bx - s, -s);
  x.lineTo(bx + s, s);
  x.moveTo(bx + s, -s);
  x.lineTo(bx - s, s);
  x.stroke();
  x.restore();
}

/**
 * Tone-on-tone laser marking as a transparent decal (55 % grey) over the
 * charging face's border. The 1024 px texture covers the flat face (29 mm on
 * the 34 mm die, 25 mm on the 30 mm one).
 * Call `update(serial)` to redraw it, e.g. once the font loads.
 */
export function makeEtching(half: number, g: DieGeometry, anisotropy: number, faces: FaceDef[]) {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1024;
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = anisotropy;
  const mesh = new THREE.Mesh(
    new THREE.PlaneGeometry(g.etch.flatMm * MM, g.etch.flatMm * MM),
    new THREE.MeshStandardMaterial({
      map: texture,
      transparent: true,
      opacity: 0.55,
      roughness: 0.7,
      metalness: 0.3,
      depthWrite: false,
    }),
  );
  const d = faces[CHARGING_FACE];
  mesh.rotation.set(...d.rot);
  mesh.position.set(...d.n).multiplyScalar(half + 0.0011);
  let last = "";
  return {
    mesh,
    update(serial: string) {
      last = serial;
      drawEtching(canvas, serial, g);
      texture.needsUpdate = true;
    },
    redraw() {
      if (last) this.update(last);
    },
  };
}
