import { useCallback, useEffect, useRef, useState } from "react";
import { DieView, DieViewHandle } from "./DieView";
import { FACE_NAMES, Pose, RollView, TipDirection } from "./protocol";
import { useSimulator } from "./useSimulator";

const POT_GLYPHS = ["←", "P", "→", "•", "•", "•"];

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
  const onFrames = useCallback((faces: Uint8Array[]) => die.current?.drawFrames(faces), []);
  const onPose = useCallback((pose: Pose) => die.current?.setPose(pose), []);
  const { state, send } = useSimulator({ onFrames, onPose });

  const [docked, setDocked] = useState(false);
  const [ble, setBle] = useState(false);
  const [reduced, setReduced] = useState(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  const [multiTurn, setMultiTurn] = useState(false);
  const [multiKey, setMultiKey] = useState(false);
  const shaking = useRef(false);

  useEffect(() => {
    send({ type: "reduced_motion", on: reduced });
  }, [reduced, send, state.connected]);

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
    if (docked) {
      setDocked(false);
      send({ type: "dock", docked: false });
    }
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
          ref={die}
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
