// Mirror of packages/simulator/server/src/protocol.rs.

/** Must match `PROTOCOL_VERSION` in the server's protocol.rs. */
export const PROTOCOL_VERSION = 5;

export const FACE_COUNT = 6;
export const FRAME_PACKET_TAG = 0x01;
/** Tag, format, width, height. */
export const FRAME_HEADER_LEN = 4;

/** How a frame packet's panels are packed (the firmware's display target). */
export type PanelFormat = "grey4" | "rgb565";

export interface FramePacket {
  format: PanelFormat;
  /** Panel pixels across (and down). */
  side: number;
  faces: Uint8Array[];
}
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
  | { type: "hello"; protocol: number; mode: string; die: number }
  /** The simulator swapped in the other die and rebooted its firmware. */
  | { type: "die"; die: number }
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
  | { type: "reduced_motion"; on: boolean }
  /** Simulate the 34 mm (96×96 grey) or 30 mm (64×64 colour) die. */
  | { type: "set_die"; die: number };

export interface Pose {
  /** Die body → world rotation, x y z w. */
  rotation: [number, number, number, number];
  /** Scene units. */
  position: [number, number, number];
}

/**
 * Split a binary frame packet: `[0x01][format][width][height]` then six
 * frames in `Face` order. Format 1 is 4 bpp grey (two pixels a byte, high
 * nibble first), 2 is RGB565 (two bytes a pixel, high byte first).
 */
export function decodeFramePacket(bytes: Uint8Array): FramePacket | null {
  if (bytes[0] !== FRAME_PACKET_TAG || bytes.length < FRAME_HEADER_LEN) return null;
  const format: PanelFormat | null = bytes[1] === 1 ? "grey4" : bytes[1] === 2 ? "rgb565" : null;
  const side = bytes[2];
  if (!format || side === 0 || bytes[3] !== side) return null;
  const frameBytes = format === "grey4" ? (side * side) / 2 : side * side * 2;
  if (bytes.length !== FRAME_HEADER_LEN + frameBytes * FACE_COUNT) return null;
  const faces = Array.from({ length: FACE_COUNT }, (_, i) =>
    bytes.subarray(FRAME_HEADER_LEN + i * frameBytes, FRAME_HEADER_LEN + (i + 1) * frameBytes),
  );
  return { format, side, faces };
}

export function decodePosePacket(buf: ArrayBuffer): Pose | null {
  if (buf.byteLength !== 1 + 7 * 4 || new Uint8Array(buf)[0] !== POSE_PACKET_TAG) return null;
  const view = new DataView(buf, 1);
  const f = (i: number) => view.getFloat32(i * 4, true);
  return { rotation: [f(0), f(1), f(2), f(3)], position: [f(4), f(5), f(6)] };
}
