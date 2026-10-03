import { useCallback, useEffect, useRef, useState } from "react";
import { DieView, DieViewHandle } from "./DieView";
import { FACE_NAMES, FramePacket, Pose, RollView, TipDirection } from "./protocol";
import { GEOMETRY, isDieMm } from "./geometry";
import { useSimulator } from "./useSimulator";
import { DEFAULT_FINISH, FINISHES, FinishKey } from "./finishes";

const POT_GLYPHS = ["←", "P", "→", "•", "•", "•"];

/** An ID-1 card (bank card, ISO/IEC 7810): 85.60 × 53.98 mm. */
const CARD_MM: [number, number] = [85.6, 53.98];
/** CSS's own idea of a millimetre (96 px to the inch); real screens vary. */
const CSS_PX_PER_MM = 96 / 25.4;
const SCALE_KEY = "sugarcube.pxPerMm";

/** The calibrated screen scale, if this browser has one. */
function loadScale(): number {
  try {
    const v = Number(localStorage.getItem(SCALE_KEY));
    return v >= 1 && v <= 12 ? v : CSS_PX_PER_MM;
  } catch {
    return CSS_PX_PER_MM;
  }
}

/** Setup label as the die shows it (SIM_SPEC C2): `d20`, `3d6`, `Pass the Pot ×2`. */
function setupLabel(r: RollView): string {
  const n = r.values.length;
  if (r.die === "pass_the_pot") return n > 1 ? `Pass the Pot ×${n}` : "Pass the Pot";
  return n > 1 ? `${n}${r.die}` : r.die;
}

function rollSummary(r: RollView): string {
  return r.die === "pass_the_pot" ? r.values.map((v) => POT_GLYPHS[v - 1]).join(" ") : String(r.total);
}

/** Tip directions and what they do (SIM_SPEC C3, "Tips"). */
const TIPS: { dir: TipDirection; arrow: string; brings: string; menu: string }[] = [
  { dir: "up", arrow: "▲", brings: "bottom screen to front", menu: "next value" },
  { dir: "left", arrow: "◀", brings: "right screen to front", menu: "next page" },
  { dir: "right", arrow: "▶", brings: "left screen to front", menu: "previous page" },
  { dir: "down", arrow: "▼", brings: "top screen to front", menu: "previous value" },
];

const KEY_TIPS: Record<string, TipDirection> = {
  ArrowUp: "up",
  ArrowDown: "down",
  ArrowLeft: "left",
  ArrowRight: "right",
};

export function App() {
  const die = useRef<DieViewHandle>(null);
  const onFrames = useCallback((frames: FramePacket) => die.current?.drawFrames(frames), []);
  const onPose = useCallback((pose: Pose) => {
    poseRef.current = pose;
    die.current?.setPose(pose);
  }, []);
  const { state, send } = useSimulator({ onFrames, onPose });

  const [nestFace, setNestFace] = useState(3); // −Y, the charging face
  const [nestQuarters, setNestQuarters] = useState(0);
  const [plugged, setPlugged] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [fault, setFault] = useState(false);
  const [stray, setStray] = useState(false);
  const [battery, setBattery] = useState(78);
  const [chargeRate, setChargeRate] = useState(1);
  const [clock, setClock] = useState(() => {
    const d = new Date();
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  });
  const poseRef = useRef<Pose | null>(null);
  const [ble, setBle] = useState(false);
  const [reduced, setReduced] = useState(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  const [multiTurn, setMultiTurn] = useState(false);
  const [multiKey, setMultiKey] = useState(false);
  const [finish, setFinish] = useState<FinishKey>(DEFAULT_FINISH);
  const [night, setNight] = useState(false);
  const [glass, setGlass] = useState(false);
  const [pixelGrid, setPixelGrid] = useState(false);
  const [closeUp, setCloseUp] = useState(false);
  const [trueSize, setTrueSize] = useState(false);
  const [pxPerMm, setPxPerMm] = useState(loadScale);
  const shaking = useRef(false);
  // The die the server simulates; the view is rebuilt when it changes.
  const geometry = GEOMETRY[isDieMm(state.die) ? state.die : 34];

  // The finish recolours the shell, contacts and (later) the Nest band, never the screens.
  useEffect(() => die.current?.setLook(finish, night), [finish, night, geometry]);
  useEffect(() => die.current?.setGlass(glass), [glass, geometry]);
  useEffect(() => die.current?.setCloseUp(closeUp), [closeUp, geometry]);
  useEffect(() => die.current?.setTrueSize(trueSize ? pxPerMm : null), [trueSize, pxPerMm, geometry]);
  useEffect(() => {
    try {
      localStorage.setItem(SCALE_KEY, String(pxPerMm));
    } catch {
      // No storage (private window): the scale lasts for this visit.
    }
  }, [pxPerMm]);
  // Etched on the charging face: the last six hex digits of the die's serial, once a roll has shown it.
  const serial = state.rolls[0]?.device_serial.slice(-6).toUpperCase() ?? "000042";
  useEffect(() => die.current?.setSerial(serial), [serial, geometry]);
  // A rebuilt view starts square to the world: put it where the die is.
  useEffect(() => {
    if (poseRef.current) die.current?.setPose(poseRef.current);
  }, [geometry]);

  useEffect(() => {
    send({ type: "reduced_motion", on: reduced });
  }, [reduced, send, state.connected]);

  // Bring the die's clock to the browser's, and the bench to the panel's
  // settings, whenever the link (re)connects.
  useEffect(() => {
    if (!state.connected) return;
    const d = new Date();
    send({ type: "set_time", seconds: d.getHours() * 3600 + d.getMinutes() * 60 + d.getSeconds() });
    send({ type: "nest_plugged", on: plugged });
    send({ type: "dirty_contacts", on: dirty });
    send({ type: "charger_fault", on: fault });
    send({ type: "stray_magnet", on: stray });
    send({ type: "battery", percent: battery });
    send({ type: "charge_rate", rate: chargeRate });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.connected, send]);

  /** The face pointing down in the current pose (the one whose normal has the lowest y). */
  const currentDownFace = () => {
    const q = poseRef.current?.rotation;
    if (!q) return 3;
    const [x, y, z, w] = q;
    // y component of q · n for each unit face normal.
    const yOf = [
      2 * (x * y + w * z), // +X
      -2 * (x * y + w * z), // −X
      1 - 2 * (x * x + z * z), // +Y
      -(1 - 2 * (x * x + z * z)), // −Y
      2 * (y * z - w * x), // +Z
      -2 * (y * z - w * x), // −Z
    ];
    return yOf.indexOf(Math.min(...yOf));
  };

  const placeInNest = () =>
    send({ type: "place_in_nest", face: nestFace < 0 ? currentDownFace() : nestFace, quarters: nestQuarters });

  const tip = useCallback(
    (dir: TipDirection) => send({ type: "tip", dir, right: die.current?.viewerRight() ?? [1, 0, 0] }),
    [send],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const dir = KEY_TIPS[e.key];
      if (dir && !(e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        tip(dir);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [tip]);

  // The throw button, as in the mockup: hold to shake, let go to throw.
  const shakeStart = () => {
    shaking.current = true;
    send({ type: "shake_start" });
  };
  const shakeEnd = (thenThrow: boolean) => {
    if (!shaking.current) return;
    shaking.current = false;
    send({ type: "shake_end", throw: thenThrow });
  };

  const menuOpen = state.mode.startsWith("Menu");
  const hapticActive = state.lastHaptic && Date.now() - state.lastHaptic.at < 400;

  return (
    <div className="layout">
      <main className={hapticActive ? "stage buzz" : "stage"}>
        <DieView
          key={geometry.mm}
          ref={die}
          geometry={geometry}
          pixelGrid={pixelGrid}
          onTouch={(face, pressed) => send({ type: "touch", face, pressed })}
          onRotate={(yaw, pitch) => send({ type: "rotate", yaw, pitch })}
          swipeToTip={menuOpen}
          onTip={tip}
          multiTurn={multiTurn}
          onSpin={(axis, angle) =>
            send({ type: "spin", axis, angle, right: die.current?.viewerRight() ?? [1, 0, 0] })
          }
          onSpinEnd={() => send({ type: "spin_end" })}
          onMultiKey={setMultiKey}
        />
        {state.protocolMismatch && (
          <div className="banner" role="alert">
            This page is out of date for the running simulator (protocol {state.protocolMismatch.page}, server{" "}
            {state.protocolMismatch.server}). Rebuild the UI with{" "}
            <code>npm run build -w @smokebomb/simulator-web-ui</code> and reload.
          </div>
        )}
        <div className="hud">
          <span className={state.connected ? "dot on" : "dot"} />
          {state.connected ? "firmware running" : "connecting to simulator…"}
          <span className="mode">{state.mode}</span>
          <span className="mode">nest: {state.nest}</span>
          {(multiKey || multiTurn) && <span className="mode multi">Multi-turn</span>}
        </div>
        <p className="hint">
          {menuOpen
            ? "Menu: swipe or use the tip pad to turn to the next screen · hold F and drag to turn several · tap to change a setting · hold to save"
            : "Drag to turn the die · press and hold a face to touch it · arrow keys tip it"}
        </p>
      </main>

      <aside className="panel">
        <section>
          <h2>Die</h2>
          <div className="chips" role="radiogroup" aria-label="Die">
            {([34, 30] as const).map((mm) => (
              <button
                key={mm}
                role="radio"
                aria-checked={geometry.mm === mm}
                className={geometry.mm === mm ? "chip on" : "chip"}
                onClick={() => send({ type: "set_die", die: mm })}
              >
                {GEOMETRY[mm].label}
              </button>
            ))}
          </div>
          <p className="muted">
            Switching reboots the firmware built for that die's panels. The 30 mm die is a proof of concept: its panel
            and window sizes are estimates until the module's drawing is in.
          </p>
          <button
            className={trueSize ? "chip on true-size" : "chip true-size"}
            aria-pressed={trueSize}
            onClick={() => setTrueSize((t) => !t)}
          >
            True size ({geometry.mm} mm)
          </button>
          {trueSize && (
            <div className="calibrate">
              <p className="muted">
                Hold a bank card upright against the outline and drag until they match, so a millimetre here is a
                millimetre on the die. Measured at the die's centre; the near edges show a little larger.
              </p>
              <div
                className="card-outline"
                // Portrait, so it fits the panel; ID-1 corners are 3.18 mm.
                style={{ width: CARD_MM[1] * pxPerMm, height: CARD_MM[0] * pxPerMm, borderRadius: 3.18 * pxPerMm }}
                aria-hidden="true"
              />
              <label>
                Screen scale {pxPerMm.toFixed(2)} px/mm
                <input
                  type="range"
                  min={2}
                  max={10}
                  step={0.01}
                  value={pxPerMm}
                  onChange={(e) => setPxPerMm(Number(e.target.value))}
                />
              </label>
              <button className="chip" onClick={() => setPxPerMm(CSS_PX_PER_MM)}>
                Reset to 96 dpi
              </button>
            </div>
          )}
          <label className="check">
            <input
              type="checkbox"
              checked={closeUp}
              disabled={trueSize}
              onChange={(e) => setCloseUp(e.target.checked)}
            />
            Close-up
          </label>
          <label className="check">
            <input type="checkbox" checked={pixelGrid} onChange={(e) => setPixelGrid(e.target.checked)} />
            Pixel gaps (for close-ups)
          </label>
        </section>

        <section>
          <h2>Motion</h2>
          <button
            className="primary throw"
            onPointerDown={(e) => {
              e.currentTarget.setPointerCapture(e.pointerId);
              shakeStart();
            }}
            onPointerUp={() => shakeEnd(true)}
            onPointerCancel={() => shakeEnd(false)}
            onKeyDown={(e) => {
              if ((e.key === " " || e.key === "Enter") && !e.repeat) {
                e.preventDefault();
                shakeStart();
              }
            }}
            onKeyUp={(e) => {
              if (e.key === " " || e.key === "Enter") {
                e.preventDefault();
                shakeEnd(true);
              }
            }}
          >
            Hold to shake, release to throw
          </button>
        </section>

        <section>
          <h2>Tip to an adjacent screen</h2>
          <p className="muted">
            A quick quarter-turn that brings the neighbouring screen to the front, as when tipping through the menu.
            Arrow keys work too.
          </p>
          <div className="tip-pad">
            {TIPS.map(({ dir, arrow, brings, menu }) => (
              <button key={dir} className={`tip-${dir}`} aria-label={`Tip ${dir}: ${brings}`} onClick={() => tip(dir)}>
                <span className="arrow">{arrow}</span>
                <span className="what">{brings}</span>
                <span className="menu-effect">{menu}</span>
              </button>
            ))}
          </div>
          <label className="check">
            <input type="checkbox" checked={multiTurn} onChange={(e) => setMultiTurn(e.target.checked)} />
            Multi-turn swipes in the menu
          </label>
          <p className="muted">
            Or hold F while you drag: the die follows your finger across as many screens as you like and
            settles on the nearest one when you let go. Each screen passed is one step.
          </p>
        </section>

        <section>
          <h2>Finish</h2>
          <div className="chips" role="radiogroup" aria-label="Finish">
            {FINISHES.map((f) => (
              <button
                key={f.key}
                role="radio"
                aria-checked={finish === f.key}
                className={finish === f.key ? "chip on" : "chip"}
                onClick={() => setFinish(f.key)}
              >
                {f.label}
              </button>
            ))}
          </div>
          <label className="check">
            <input type="checkbox" checked={night} onChange={(e) => setNight(e.target.checked)} />
            Night (lights off)
          </label>
          <label className="check">
            <input type="checkbox" checked={glass} onChange={(e) => setGlass(e.target.checked)} />
            Sapphire glass reflections
          </label>
          <p className="muted">
            Off shows the screens unlit, so their levels (16 grey, or 65k colours) are exact. On adds the glass's
            reflections.
          </p>
        </section>

        <section>
          <h2>Set down</h2>
          <label>Face up</label>
          <div className="row faces">
            {FACE_NAMES.map((name, i) => (
              <button key={name} onClick={() => send({ type: "place_face_up", face: i })}>
                {name}
              </button>
            ))}
          </div>
        </section>

        <section>
          <h2>Nest</h2>
          <p className="muted">
            The charging face is −Y, the lid. It charges in all four rotations. Guidance appears only once the die is
            seated.
          </p>
          <label>
            Face down
            <select value={nestFace} onChange={(e) => setNestFace(Number(e.target.value))}>
              <option value={-1}>Current down face</option>
              {FACE_NAMES.map((name, i) => (
                <option key={name} value={i}>
                  {name}
                  {i === 3 ? " (charging face)" : ""}
                </option>
              ))}
            </select>
          </label>
          <label>
            Rotation
            <select value={nestQuarters} onChange={(e) => setNestQuarters(Number(e.target.value))}>
              {[0, 1, 2, 3].map((q) => (
                <option key={q} value={q}>
                  {q * 90}°
                </option>
              ))}
            </select>
          </label>
          <div className="row">
            <button className="primary" onClick={placeInNest}>
              Place in Nest
            </button>
            <button onClick={() => send({ type: "lift" })}>Lift</button>
          </div>
          <label className="check">
            <input
              type="checkbox"
              checked={plugged}
              onChange={(e) => {
                setPlugged(e.target.checked);
                send({ type: "nest_plugged", on: e.target.checked });
              }}
            />
            Nest plugged in
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={dirty}
              onChange={(e) => {
                setDirty(e.target.checked);
                send({ type: "dirty_contacts", on: e.target.checked });
              }}
            />
            Dirty contacts
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={fault}
              onChange={(e) => {
                setFault(e.target.checked);
                send({ type: "charger_fault", on: e.target.checked });
              }}
            />
            Charger fault
          </label>
          <label className="check">
            <input
              type="checkbox"
              checked={stray}
              onChange={(e) => {
                setStray(e.target.checked);
                send({ type: "stray_magnet", on: e.target.checked });
              }}
            />
            Stray magnet beside the die
          </label>
          <label>
            Battery {battery}%
            <input
              type="range"
              min={0}
              max={100}
              value={battery}
              onChange={(e) => {
                setBattery(Number(e.target.value));
                send({ type: "battery", percent: Number(e.target.value) });
              }}
            />
          </label>
          <label>
            Charge rate
            <select
              value={chargeRate}
              onChange={(e) => {
                setChargeRate(Number(e.target.value));
                send({ type: "charge_rate", rate: Number(e.target.value) });
              }}
            >
              {[1, 10, 60, 600].map((r) => (
                <option key={r} value={r}>
                  ×{r} ({r} %/min)
                </option>
              ))}
            </select>
          </label>
          <label>
            Die's clock (local time)
            <input
              type="time"
              value={clock}
              onChange={(e) => {
                setClock(e.target.value);
                const [h, m] = e.target.value.split(":").map(Number);
                if (!Number.isNaN(h) && !Number.isNaN(m)) send({ type: "set_time", seconds: h * 3600 + m * 60 });
              }}
            />
          </label>
        </section>

        <section>
          <h2>Environment</h2>
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
          <label className="check">
            <input type="checkbox" checked={reduced} onChange={(e) => setReduced(e.target.checked)} />
            Reduced motion
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
                  <strong>{rollSummary(r)}</strong>
                  <span>
                    {setupLabel(r)} [{r.values.join(", ")}]
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
