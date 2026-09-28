import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { DieView, DieViewHandle, FACE_NORMALS } from "./DieView";
import { FACE_NAMES, Gesture } from "./protocol";
import { useSimulator } from "./useSimulator";

const WORLD_UP = new THREE.Vector3(0, 1, 0);

/** Body -> world rotation for a given up face plus a small tilt (degrees). */
function orientation(upFace: number, pitch: number, roll: number): THREE.Quaternion {
  const base = new THREE.Quaternion().setFromUnitVectors(FACE_NORMALS[upFace], WORLD_UP);
  const tilt = new THREE.Quaternion().setFromEuler(
    new THREE.Euler(THREE.MathUtils.degToRad(pitch), 0, THREE.MathUtils.degToRad(roll)),
  );
  return tilt.multiply(base);
}

/** What the accelerometer reads at rest: world up expressed in the body frame, in mg. */
function gravityMg(q: THREE.Quaternion): [number, number, number] {
  const g = WORLD_UP.clone().applyQuaternion(q.clone().invert()).multiplyScalar(1000);
  return [Math.round(g.x), Math.round(g.y), Math.round(g.z)];
}

export function App() {
  const die = useRef<DieViewHandle>(null);
  const onFrames = useCallback((faces: Uint8Array[]) => die.current?.drawFrames(faces), []);
  const { state, send } = useSimulator(onFrames);

  const [upFace, setUpFace] = useState(4); // +Z
  const [pitch, setPitch] = useState(0);
  const [roll, setRoll] = useState(0);
  const [docked, setDocked] = useState(false);
  const [ble, setBle] = useState(false);
  const tumbleNext = useRef(false);

  const q = useMemo(() => orientation(upFace, pitch, roll), [upFace, pitch, roll]);

  // Keep the 3D view and the simulated accelerometer in sync with orientation.
  useEffect(() => {
    die.current?.setOrientation(q, tumbleNext.current);
    tumbleNext.current = false;
    send({ type: "imu", accel: gravityMg(q), gyro: [0, 0, 0] });
  }, [q, send, state.connected]);

  const gesture = (kind: Gesture) => {
    if (kind !== "throw") {
      send({ type: "gesture", kind });
      return;
    }
    const land = Math.floor(Math.random() * 6);
    send({ type: "gesture", kind, land });
    tumbleNext.current = true;
    setPitch(0);
    setRoll(0);
    setUpFace(land);
  };

  const hapticActive = state.lastHaptic && Date.now() - state.lastHaptic.at < 400;

  return (
    <div className="layout">
      <main className={hapticActive ? "stage buzz" : "stage"}>
        <DieView ref={die} onTouch={(face, pressed) => send({ type: "touch", face, pressed })} />
        <div className="hud">
          <span className={state.connected ? "dot on" : "dot"} />
          {state.connected ? "firmware running" : "connecting to simulator…"}
          <span className="mode">{state.mode}</span>
        </div>
        <p className="hint">Drag to orbit · press and hold a face to touch it (hold 1.5 s for the menu)</p>
      </main>

      <aside className="panel">
        <section>
          <h2>Motion</h2>
          <div className="row">
            <button onClick={() => gesture("pick_up")}>Pick up</button>
            <button onClick={() => gesture("shake")}>Shake</button>
            <button className="primary" onClick={() => gesture("throw")}>
              Throw
            </button>
          </div>
        </section>

        <section>
          <h2>Orientation</h2>
          <label>Face up</label>
          <div className="row faces">
            {FACE_NAMES.map((name, i) => (
              <button key={name} className={i === upFace ? "selected" : ""} onClick={() => setUpFace(i)}>
                {name}
              </button>
            ))}
          </div>
          <label>
            Tilt X <output>{pitch}°</output>
          </label>
          <input type="range" min={-60} max={60} value={pitch} onChange={(e) => setPitch(+e.target.value)} />
          <label>
            Tilt Z <output>{roll}°</output>
          </label>
          <input type="range" min={-60} max={60} value={roll} onChange={(e) => setRoll(+e.target.value)} />
        </section>

        <section>
          <h2>Environment</h2>
          <label className="check">
            <input
              type="checkbox"
              checked={docked}
              onChange={(e) => {
                setDocked(e.target.checked);
                send({ type: "dock", docked: e.target.checked });
              }}
            />
            On charging nest
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={ble}
              onChange={(e) => {
                setBle(e.target.checked);
                send({ type: "ble", connected: e.target.checked });
              }}
            />
            Phone connected (BLE)
          </label>
          <p className="muted">Last haptic: {state.lastHaptic?.effect ?? "—"}</p>
        </section>

        <section className="rolls">
          <h2>Signed rolls</h2>
          {state.rolls.length === 0 && <p className="muted">Throw the die to roll.</p>}
          <ol>
            {state.rolls.map((r) => (
              <li key={r.counter}>
                <div className="roll-head">
                  <strong>{r.total}</strong>
                  <span>
                    {r.values.length}d{r.die_sides} [{r.values.join(", ")}]
                  </span>
                  <span className="muted">#{r.counter}</span>
                </div>
                <code title={`digest ${r.digest}\nsignature ${r.signature}`}>
                  {r.digest.slice(0, 16)}… sig {r.signature.slice(0, 12)}…
                </code>
              </li>
            ))}
          </ol>
        </section>
      </aside>
    </div>
  );
}
