// The two dice the simulator can show, in millimetres. Mirrors the server's
// `DieSize` and the firmware's display targets (`smokebomb_hal::target`).

/** Scene units per mm: 1 u = 13.25 mm, as in the mockup (SIM_SPEC A1). */
export const MM = 1 / 13.25;

export type DieMm = 34 | 30;

export interface DieGeometry {
  mm: DieMm;
  /** Chip label in the Die section. */
  label: string;
  edgeMm: number;
  /** Glass window: half its side and its corner radius. */
  windowHalfMm: number;
  windowRadiusMm: number;
  /** Panel pixels across. */
  panelPx: number;
  /**
   * The screen texture spans ±1 u (26.5 mm) of the face, `texture` texels
   * across; each panel pixel is `texelsPerPx` texels (a whole number, so the
   * nearest-neighbour upscale keeps every pixel the same size). The lit area
   * is therefore `panelPx × texelsPerPx / texture × 26.5` mm.
   */
  texture: number;
  texelsPerPx: number;
  /** The glass's ink mask rounds the lit area by this many texels. */
  maskTexels: number;
  /** Charging contacts (the lid's screws). */
  screws:
    | { kind: "hex"; atMm: number }
    | { kind: "slotted"; atMm: number; headMm: number; ringMm: number; slotMm: number; slotAngles: number[] };
  /** Laser etching round the charging face's window. */
  etch: {
    /** The flat face: die less two edge radii. */
    flatMm: number;
    /** Where the border band starts (the window's half-size) and its centre line. */
    windowHalfMm: number;
    bandMm: number;
    /** Script lines centred in the band (30 mm) or hugging the window (34 mm). */
    centreScripts: boolean;
  };
}

export const GEOMETRY: Record<DieMm, DieGeometry> = {
  // SIM_SPEC A1–A3: 24 mm window (r 2.52), the 96×96 panel 3 texels a pixel
  // in a 444-texel texture (17.19 mm drawn for 17.26 mm), mask 42 texels.
  34: {
    mm: 34,
    label: "34 mm · 96×96 grey",
    edgeMm: 2.5,
    windowHalfMm: 0.906 / MM,
    windowRadiusMm: 0.19 / MM,
    panelPx: 96,
    texture: 444,
    texelsPerPx: 3,
    maskTexels: 42,
    screws: { kind: "hex", atMm: 13.2 },
    etch: { flatMm: 29, windowHalfMm: 12, bandMm: 13.25, centreScripts: false },
  },
  // The mockup's 30 mm option: a 17.5 mm window (r 1.84) round a 10.75 mm
  // lit square (estimates until the NHD-0.6-6464G drawing). 4 texels a pixel
  // in a 632-texel texture draws the lit area at 256/632 × 26.5 = 10.73 mm.
  // The mask is 10.7 panel px ≈ 1.8 mm, as the mockup's. Four plain
  // single-slot screws at ±10.625 mm, centred between the window (8.75) and
  // the edge radius (12.5), each stopped where its torque left it.
  30: {
    mm: 30,
    label: "30 mm · 64×64 colour",
    edgeMm: 2.5,
    windowHalfMm: 8.75,
    windowRadiusMm: 1.84,
    panelPx: 64,
    texture: 632,
    texelsPerPx: 4,
    maskTexels: 43,
    screws: {
      kind: "slotted",
      atMm: 10.625,
      headMm: 1.6,
      ringMm: 0.96,
      slotMm: 0.25,
      slotAngles: [0.35, 1.9, 2.7, 0.95],
    },
    etch: { flatMm: 25, windowHalfMm: 8.75, bandMm: (8.75 + 12.5) / 2, centreScripts: true },
  },
};

export const isDieMm = (v: number): v is DieMm => v === 34 || v === 30;
