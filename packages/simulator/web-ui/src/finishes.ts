// Finish (colour) data from the Smokebomb design brief. This page runs three.js
// with colour management on, so hex values here are the "sRGB equivalent"
// column (the mockup's r128 fed the linear values straight into the shader).
// Screens are never tinted by a finish.

export type FinishKey =
  | "stealth"
  | "chrome"
  | "ceramic"
  | "glow"
  | "gold"
  | "rainbow";

export interface Finish {
  key: FinishKey;
  label: string;
  /** sRGB hex. */
  color: number;
  metalness: number;
  roughness: number;
  envMapIntensity: number;
  /** Emissive sRGB hex (glow ceramic). */
  emissive?: number;
  /** Plating of the four charging-face contacts (brief §4). sRGB hex. */
  contact: { color: number; metalness: number; roughness: number };
  /** Per-pixel heat-tint shader (titanium). */
  heatTint?: boolean;
}

/** In the order of the selector. The first is the default. */
export const FINISHES: Finish[] = [
  {
    key: "stealth",
    label: "Stealth black",
    color: 0x68696e,
    metalness: 0.9,
    roughness: 0.38,
    envMapIntensity: 1,
    contact: { color: 0x76777c, metalness: 1, roughness: 0.3 },
  },
  {
    key: "chrome",
    label: "Polished chrome",
    color: 0xccd0d4,
    metalness: 1,
    roughness: 0.04,
    envMapIntensity: 1,
    contact: { color: 0xcdd1d5, metalness: 1, roughness: 0.06 },
  },
  {
    key: "ceramic",
    label: "White ceramic",
    color: 0xf9f8f6,
    metalness: 0,
    roughness: 0.22,
    envMapIntensity: 1,
    contact: { color: 0x76777c, metalness: 1, roughness: 0.3 },
  },
  {
    key: "glow",
    label: "Glow ceramic",
    color: 0xf4f7f4,
    metalness: 0,
    roughness: 0.3,
    envMapIntensity: 1,
    contact: { color: 0x76777c, metalness: 1, roughness: 0.3 },
    emissive: 0xbaffe3,
  },
  // 18k yellow gold: physical reflectance, with stronger reflections.
  {
    key: "gold",
    label: "Gold (18k yellow)",
    color: 0xfbd48b,
    metalness: 1,
    roughness: 0.18,
    envMapIntensity: 1.35,
    contact: { color: 0xf9d8a5, metalness: 1, roughness: 0.16 },
  },
  {
    key: "rainbow",
    label: "Heat-tinted titanium",
    color: 0xefeff0,
    metalness: 1,
    roughness: 0.22,
    envMapIntensity: 1,
    contact: { color: 0xe5e6e7, metalness: 1, roughness: 0.12 },
    heatTint: true,
  },
];

export const DEFAULT_FINISH: FinishKey = "stealth";

/** Glow ceramic's emissive intensity by lighting. */
export const GLOW_INTENSITY = { day: 0.05, night: 1.1 };

/** Day and night lighting: tone-mapping exposure, key light and ambient. */
export const LIGHTING = {
  day: { exposure: 0.8, key: 0.45, ambient: 0.12 },
  night: { exposure: 0.32, key: 0.05, ambient: 0.02 },
};

/** Contact gap ring and slot, the lid seam and the laser etching (SIM brief §1, §2). */
export const SCREW_DARK = { color: 0x38383b, roughness: 0.8, metalness: 0.2 };
