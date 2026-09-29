// Mirror of packages/simulator/server/src/protocol.rs.

/** Must match `PROTOCOL_VERSION` in the server's protocol.rs. */
export const PROTOCOL_VERSION = 4;

export const FACE_COUNT = 6;
export const PANEL_SIZE = 96;
export const FRAME_BYTES = (PANEL_SIZE * PANEL_SIZE) / 2; // 4bpp
export const FRAME_PACKET_TAG = 0x01;
export const POSE_PACKET_TAG = 0x02;

/** Face order matches three.js BoxGeometry material order and the firmware `Face` enum. */
export const FACE_NAMES = ["+X", "−X", "+Y", "−Y", "+Z", "−Z"] as const;

export interface RollView {
  device_serial: string;
  counter: number;
  uptime_ms: number;
  /** `d4` … `d100` or `pass_the_pot` */
  die: string;
  values: number[];
  total: number;
  digest: string;
  prev_hash: string;
  signature: string;
}

export type ServerEvent =
  | { type: "hello"; protocol: number; mode: string }
  | { type: "mode"; mode: string }
  | { type: "haptic"; effect: string }
  /** The Nest's dock state: OffNest, Seating, Ok, Wrong, NoPower or Display. */
  | { type: "nest"; phase: string }
  | ({ type: "roll" } & RollView);

export type TipDirection = "up" | "down" | "left" | "right";

/** Multi-turn spin axis: yaw (left/right tips) or pitch (up/down tips). */
export type SpinAxis = "yaw" | "pitch";

export type ClientMessage =
  | { type: "touch"; face: number; pressed: boolean }
  | { type: "shake_start" }
  | { type: "shake_end"; throw: boolean }
  | { type: "tip"; dir: TipDirection; right: [number, number, number] }
  | { type: "spin"; axis: SpinAxis; angle: number; right: [number, number, number] }
  | { type: "spin_end" }
  | { type: "rotate"; yaw: number; pitch: number }
  | { type: "place_face_up"; face: number }
  | { type: "dock"; docked: boolean }
  | { type: "place_in_nest"; face: number; quarters: number }
  | { type: "lift" }
  | { type: "stray_magnet"; on: boolean }
  | { type: "nest_plugged"; on: boolean }
  | { type: "dirty_contacts"; on: boolean }
  | { type: "charger_fault"; on: boolean }
  | { type: "battery"; percent: number }
  | { type: "charge_rate"; rate: number }
  | { type: "set_time"; seconds: number }
  | { type: "ble"; connected: boolean }
  | { type: "reduced_motion"; on: boolean };

export interface Pose {
  /** Die body → world rotation, x y z w. */
  rotation: [number, number, number, number];
  /** Scene units. */
  position: [number, number, number];
}

/** Split a binary frame packet into six packed 4bpp panel frames. */
export function decodeFramePacket(bytes: Uint8Array): Uint8Array[] | null {
  if (bytes[0] !== FRAME_PACKET_TAG || bytes.length !== 1 + FRAME_BYTES * FACE_COUNT) return null;
  return Array.from({ length: FACE_COUNT }, (_, i) =>
    bytes.subarray(1 + i * FRAME_BYTES, 1 + (i + 1) * FRAME_BYTES),
  );
}

export function decodePosePacket(buf: ArrayBuffer): Pose | null {
  if (buf.byteLength !== 1 + 7 * 4 || new Uint8Array(buf)[0] !== POSE_PACKET_TAG) return null;
  const view = new DataView(buf, 1);
  const f = (i: number) => view.getFloat32(i * 4, true);
  return { rotation: [f(0), f(1), f(2), f(3)], position: [f(4), f(5), f(6)] };
}
