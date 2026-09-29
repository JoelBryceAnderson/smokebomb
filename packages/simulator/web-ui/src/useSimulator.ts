import { useCallback, useEffect, useRef, useState } from "react";
import {
  ClientMessage,
  decodeFramePacket,
  decodePosePacket,
  FRAME_PACKET_TAG,
  Pose,
  PROTOCOL_VERSION,
  RollView,
  ServerEvent,
} from "./protocol";

export interface SimulatorState {
  connected: boolean;
  /** The server speaks a different protocol version than this page. */
  protocolMismatch: { server: number; page: number } | null;
  mode: string;
  /** The Nest's dock state. */
  nest: string;
  rolls: RollView[];
  lastHaptic: { effect: string; at: number } | null;
}

export interface Streams {
  /** Six packed 4bpp frames, at up to the firmware rate. */
  onFrames(faces: Uint8Array[]): void;
  /** The die's pose from the server's world model, every tick it moves. */
  onPose(pose: Pose): void;
}

/**
 * WebSocket link to the simulator server. Frames and poses bypass React state
 * and go straight to the 3D view.
 */
export function useSimulator(streams: Streams) {
  const [state, setState] = useState<SimulatorState>({
    connected: false,
    protocolMismatch: null,
    mode: "—",
    nest: "OffNest",
    rolls: [],
    lastHaptic: null,
  });
  const socket = useRef<WebSocket | null>(null);
  const cb = useRef(streams);
  cb.current = streams;

  useEffect(() => {
    let closed = false;
    let retry: ReturnType<typeof setTimeout>;

    const connect = () => {
      const proto = location.protocol === "https:" ? "wss" : "ws";
      const ws = new WebSocket(`${proto}://${location.host}/ws`);
      ws.binaryType = "arraybuffer";
      socket.current = ws;

      ws.onopen = () => setState((s) => ({ ...s, connected: true }));
      ws.onclose = () => {
        setState((s) => ({ ...s, connected: false }));
        if (!closed) retry = setTimeout(connect, 1000);
      };
      ws.onmessage = (msg) => {
        if (msg.data instanceof ArrayBuffer) {
          const bytes = new Uint8Array(msg.data);
          if (bytes[0] === FRAME_PACKET_TAG) {
            const faces = decodeFramePacket(bytes);
            if (faces) cb.current.onFrames(faces);
          } else {
            const pose = decodePosePacket(msg.data);
            if (pose) cb.current.onPose(pose);
          }
          return;
        }
        const ev = JSON.parse(msg.data as string) as ServerEvent;
        setState((s) => {
          switch (ev.type) {
            case "hello":
              return {
                ...s,
                mode: ev.mode || s.mode,
                protocolMismatch:
                  ev.protocol === PROTOCOL_VERSION ? null : { server: ev.protocol, page: PROTOCOL_VERSION },
              };
            case "mode":
              return { ...s, mode: ev.mode };
            case "nest":
              return { ...s, nest: ev.phase };
            case "haptic":
              return { ...s, lastHaptic: { effect: ev.effect, at: Date.now() } };
            case "roll": {
              const { type: _type, ...roll } = ev;
              return { ...s, rolls: [roll, ...s.rolls].slice(0, 50) };
            }
          }
        });
      };
    };

    connect();
    return () => {
      closed = true;
      clearTimeout(retry);
      socket.current?.close();
    };
  }, []);

  const send = useCallback((msg: ClientMessage) => {
    if (socket.current?.readyState === WebSocket.OPEN) socket.current.send(JSON.stringify(msg));
  }, []);

  return { state, send };
}
