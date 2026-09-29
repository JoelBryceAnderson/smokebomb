// Corner screws and the charging-face etching (design brief §1 and §2).
import * as THREE from "three";

const MM = 1 / 13.25; // scene units per mm

/** Screw centre from the face centre, in mm, on both axes (the updated mockup: 12.9 mm, 1.9 mm heads). */
const SCREW_AT = 12.9 * MM;
const HEX_R = 1.1 * MM; // 1.9 mm across flats
const HEX_GAP = 0.15 * MM;

export interface FaceDef {
  n: [number, number, number];
  rot: [number, number, number];
}

/**
 * Four hexagonal screws per face. The heads are flush with the shell, so
 * they take its finish for free; what is drawn here is the dark hairline
 * ring around each head and the slot across it, both flush inlays. All
 * slots point at the window centre, and a hex flat faces it too.
 * On the five cup faces they are engraved; on the charging face (the lid) the
 * slot is an insulating gap splitting each head into a + half and a ground
 * half, and the Nest's two pins straddle it, so every rotation lines up.
 */
export function addScrews(die: THREE.Group, faces: FaceDef[], half: number, material: THREE.Material) {
  const ringGeo = new THREE.RingGeometry(HEX_R, HEX_R + HEX_GAP, 6, 1);
  const slotGeo = new THREE.PlaneGeometry(1.8 * MM, 0.2 * MM);
  for (const d of faces) {
    for (const [sx, sy] of [
      [1, 1],
      [1, -1],
      [-1, 1],
      [-1, -1],
    ]) {
      const g = new THREE.Group();
      const ring = new THREE.Mesh(ringGeo, material);
      ring.rotation.z = Math.atan2(-sy, -sx) - Math.PI / 6;
      const slot = new THREE.Mesh(slotGeo, material);
      slot.rotation.z = Math.atan2(-sy, -sx);
      slot.position.z = 0.0002;
      g.add(ring, slot);
      g.rotation.set(...d.rot);
      // ~0.01 mm above the metal (0.00075 u) to stay clear of z-fighting.
      g.position.set(...d.n).multiplyScalar(half + 0.0009);
      g.translateX(sx * SCREW_AT);
      g.translateY(sy * SCREW_AT);
      die.add(g);
    }
  }
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
  top: "SMOKEBOMB",
  bottom: "DESIGNED IN BROOKLYN  ·  BUILT TO BE THROWN",
  left: "REGULATORY INFO IN SETTINGS",
};

function drawEtching(c: HTMLCanvasElement, serial: string) {
  const C = c.width;
  const PXMM = C / 29;
  const x = c.getContext("2d")!;
  x.clearRect(0, 0, C, C);
  x.fillStyle = x.strokeStyle = "rgb(150,152,156)";
  x.textAlign = "center";
  x.textBaseline = "middle";
  const band = C / 2 - 13.25 * PXMM; // centre line of the 2.5 mm border
  const font = (mm: number) => `600 ${mm * PXMM}px "Space Grotesk", Arial, sans-serif`;
  const side = (rot: number, text: string, mm: number) => {
    x.save();
    x.translate(C / 2, C / 2);
    x.rotate(rot);
    x.font = font(mm);
    x.fillText(text, 0, -(C / 2 - band));
    x.restore();
  };
  side(0, ETCH_TEXT.top, 0.9);
  side(Math.PI / 2, `SB-1  ·  S/N ${serial}`, 0.7);
  side(Math.PI, ETCH_TEXT.bottom, 0.62); // room is left here for a "Made in …" line later
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
 * charging face's border. The 1024 px texture covers the 29 mm flat face.
 * Call `update(serial)` to redraw it, e.g. once the font loads.
 */
export function makeEtching(half: number, anisotropy: number, faces: FaceDef[]) {
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = 1024;
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = anisotropy;
  const mesh = new THREE.Mesh(
    new THREE.PlaneGeometry(29 * MM, 29 * MM),
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
      drawEtching(canvas, serial);
      texture.needsUpdate = true;
    },
    redraw() {
      if (last) this.update(last);
    },
  };
}
