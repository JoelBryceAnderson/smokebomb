import { useCallback, useEffect, useRef, useState } from "react";
import { ClientMessage, decodeFramePacket, RollView, ServerEvent } from "./protocol";

export interface SimulatorState {
  connected: boolean;
  mode: string;
  rolls: RollView[];
  lastHaptic: { effect: string; at: number } | null;
}

/**
 * WebSocket link to the simulator server. Frames bypass React state and go
 * straight to `onFrames` (called at up to 30 Hz).
 */
export function useSimulator(onFrames: (faces: Uint8Array[]) => void) {
  const [state, setState] = useState<SimulatorState>({
    connected: false,
    mode: "—",
    rolls: [],
    lastHaptic: null,
  });
  const socket = useRef<WebSocket | null>(null);
  const framesCb = useRef(onFrames);
  framesCb.current = onFrames;

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
          const faces = decodeFramePacket(msg.data);
          if (faces) framesCb.current(faces);
          return;
        }
        const ev = JSON.parse(msg.data as string) as ServerEvent;
        setState((s) => {
          switch (ev.type) {
            case "mode":
              return { ...s, mode: ev.mode };
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
