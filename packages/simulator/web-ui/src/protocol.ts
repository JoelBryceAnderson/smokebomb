// Mirror of packages/simulator/server/src/protocol.rs.

export const FACE_COUNT = 6;
export const PANEL_SIZE = 96;
export const FRAME_BYTES = (PANEL_SIZE * PANEL_SIZE) / 2; // 4bpp
export const FRAME_PACKET_TAG = 0x01;

/** Face order matches three.js BoxGeometry material order and the firmware `Face` enum. */
export const FACE_NAMES = ["+X", "−X", "+Y", "−Y", "+Z", "−Z"] as const;

export interface RollView {
  device_serial: string;
  counter: number;
  uptime_ms: number;
  die_sides: number;
  values: number[];
  total: number;
  digest: string;
  prev_hash: string;
  signature: string;
}

export type ServerEvent =
  | { type: "mode"; mode: string }
  | { type: "haptic"; effect: string }
  | ({ type: "roll" } & RollView);

export type Gesture = "pick_up" | "shake" | "throw";

export type ClientMessage =
  | { type: "touch"; face: number; pressed: boolean }
  | { type: "orient"; up: number }
  | { type: "imu"; accel: [number, number, number]; gyro: [number, number, number] }
  | { type: "gesture"; kind: Gesture; land?: number }
  | { type: "dock"; docked: boolean }
  | { type: "ble"; connected: boolean };

/** Split a binary frame packet into six packed 4bpp panel frames. */
export function decodeFramePacket(buf: ArrayBuffer): Uint8Array[] | null {
  const bytes = new Uint8Array(buf);
  if (bytes[0] !== FRAME_PACKET_TAG || bytes.length !== 1 + FRAME_BYTES * FACE_COUNT) return null;
  return Array.from({ length: FACE_COUNT }, (_, i) =>
    bytes.subarray(1 + i * FRAME_BYTES, 1 + (i + 1) * FRAME_BYTES),
  );
}
