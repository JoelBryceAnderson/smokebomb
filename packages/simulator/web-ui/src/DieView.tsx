import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import * as THREE from "three";
import { RoundedBoxGeometry } from "three/examples/jsm/geometries/RoundedBoxGeometry.js";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";
import { PANEL_SIZE, Pose, SpinAxis, TipDirection } from "./protocol";
import { FINISHES, FinishKey, GLOW_INTENSITY, LIGHTING, SCREW_DARK } from "./finishes";
import { addScrews, addSeam, makeEtching, ENGRAVE_GLSL, ENGRAVE_NORMAL } from "./shell";

// Mockup scale (SIM_SPEC A1): 1 scene unit = 13.25 mm, 34 mm die, 2.5 mm edges.
const MM = 1 / 13.25;
export const HALF = 17 * MM; // 1.283 u
const EDGE_RADIUS = 2.5 * MM;
/** Growth factor from the mockup's earlier 29.4 mm body; camera distances use it. */
const K = HALF / 1.11;
/** Glass window: 24 mm square, 2.52 mm corners (half-size 0.906 u, radius 0.19 u). */
const WINDOW_HALF = 0.906;
const WINDOW_RADIUS = 0.19;

// Each face's mesh rotation, as in the mockup, so canvas axes match the
// firmware's `orientation::BASES`.
const FACE_DEFS: { n: [number, number, number]; rot: [number, number, number] }[] = [
  { n: [1, 0, 0], rot: [0, Math.PI / 2, 0] },
  { n: [-1, 0, 0], rot: [0, -Math.PI / 2, 0] },
  { n: [0, 1, 0], rot: [-Math.PI / 2, 0, 0] },
  { n: [0, -1, 0], rot: [Math.PI / 2, 0, 0] },
  { n: [0, 0, 1], rot: [0, 0, 0] },
  { n: [0, 0, -1], rot: [0, Math.PI, 0] },
];

// Screen texture (SIM_SPEC A3): 444 px spanning ±1 u; the 96×96 panel fills
// the centred 288 px at 3 px per pixel; the glass's ink mask rounds it (42 px).
const OUT_T = 444;
const OUT_PX = 3;
const OUT_OFF = (OUT_T - PANEL_SIZE * OUT_PX) / 2;
const MASK_RADIUS = 42;

export interface DieViewHandle {
  /** Paint six packed 4bpp frames onto the faces. */
  drawFrames(faces: Uint8Array[]): void;
  setPose(pose: Pose): void;
  /** The viewer's right in world space (the axis for up/down tips). */
  viewerRight(): [number, number, number];
  /** Shell finish and lighting; screens are never tinted. */
  setLook(finish: FinishKey, night: boolean): void;
  /** The device serial etched on the charging face. */
  setSerial(serial: string): void;
  /** Sapphire glass over the screens (reflective PBR) instead of the exact unlit screen. */
  setGlass(on: boolean): void;
}

interface Props {
  onTouch(face: number, pressed: boolean): void;
  /** Drag deltas in radians: yaw about world Y, pitch about world X. */
  onRotate(yaw: number, pitch: number): void;
  /** While the menu is open, a swipe tips the die instead of turning it (as in the mockup). */
  swipeToTip: boolean;
  onTip(dir: TipDirection): void;
  /**
   * Multi-turn: in the menu, a drag spins the die about one tip axis and
   * follows the pointer across as many faces as you like; letting go settles
   * it on the nearest face. On with the checkbox, or while the F key is held.
   */
  multiTurn: boolean;
  /** Radians since the spin began: positive yaw is a right tip, negative pitch an up tip. */
  onSpin(axis: SpinAxis, angle: number): void;
  onSpinEnd(): void;
  /** The multi-turn key (F) went down or up. */
  onMultiKey(held: boolean): void;
}

/** Swipe distance that counts as a tip (SIM_SPEC C3). */
const SWIPE_PX = 36;
/** Multi-turn spin rate: a quarter turn per ~120 px of drag. */
const SPIN_RAD_PER_PX = 0.013;

function roundRectPath(c: CanvasRenderingContext2D | THREE.Path, x: number, y: number, w: number, h: number, r: number) {
  c.moveTo(x + r, y);
  c.lineTo(x + w - r, y);
  c.quadraticCurveTo(x + w, y, x + w, y + r);
  c.lineTo(x + w, y + h - r);
  c.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
  c.lineTo(x + r, y + h);
  c.quadraticCurveTo(x, y + h, x, y + h - r);
  c.lineTo(x, y + r);
  c.quadraticCurveTo(x, y, x + r, y);
}

export const DieView = forwardRef<DieViewHandle, Props>(function DieView(
  { onTouch, onRotate, swipeToTip, onTip, multiTurn, onSpin, onSpinEnd, onMultiKey },
  ref,
) {
  const mountRef = useRef<HTMLDivElement>(null);
  const api = useRef<DieViewHandle | null>(null);
  const cbs = useRef({ onTouch, onRotate, swipeToTip, onTip, multiTurn, onSpin, onSpinEnd, onMultiKey });
  cbs.current = { onTouch, onRotate, swipeToTip, onTip, multiTurn, onSpin, onSpinEnd, onMultiKey };

  useImperativeHandle(ref, () => ({
    drawFrames: (f) => api.current?.drawFrames(f),
    setPose: (p) => api.current?.setPose(p),
    viewerRight: () => api.current?.viewerRight() ?? [1, 0, 0],
    setLook: (f, n) => api.current?.setLook(f, n),
    setSerial: (x) => api.current?.setSerial(x),
    setGlass: (on) => api.current?.setGlass(on),
  }));

  useEffect(() => {
    const mount = mountRef.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 0.8;
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x0d0d10);
    // Placeholder studio; the mockup's exact softbox environment comes with the visual pass.
    const pmrem = new THREE.PMREMGenerator(renderer);
    scene.environment = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;

    const camera = new THREE.PerspectiveCamera(32, 1, 0.1, 100);
    const look = new THREE.Vector3();

    const die = new THREE.Group();
    scene.add(die);
    // The shell takes its finish from `setLook` (design brief §4). Env
    // reflection is per material, so give it the environment explicitly.
    const envMap = scene.environment;
    const shellMat = new THREE.MeshStandardMaterial({ envMap });
    // Heat-tinted titanium: a per-pixel tint after normal mapping that shifts
    // with viewing angle and with the die's orientation.
    const heat = { on: { value: 0 }, shift: { value: 0 } };
    shellMat.onBeforeCompile = (sh) => {
      sh.uniforms.heatOn = heat.on;
      sh.uniforms.heatShift = heat.shift;
      sh.vertexShader = sh.vertexShader
        .replace("#include <common>", "#include <common>\nvarying vec3 vHeatPos;\nvarying vec3 vObjPos;\nvarying vec3 vObjNrm;")
        .replace(
          "#include <begin_vertex>",
          "#include <begin_vertex>\nvHeatPos = (modelMatrix * vec4(transformed, 1.0)).xyz;\nvObjPos = transformed;\nvObjNrm = normal;",
        );
      sh.fragmentShader = sh.fragmentShader
        .replace(
          "#include <common>",
          `#include <common>
          varying vec3 vHeatPos;
          varying vec3 vObjPos;
          varying vec3 vObjNrm;
          uniform float heatOn;
          uniform float heatShift;
          ${ENGRAVE_GLSL}`,
        )
        .replace(
          "#include <normal_fragment_maps>",
          `#include <normal_fragment_maps>
          ${ENGRAVE_NORMAL}
          if (heatOn > 0.5) {
            float ndv = clamp(dot(normalize(normal), normalize(vViewPosition)), 0.0, 1.0);
            float t = fract(ndv * 0.9 + heatShift + dot(vHeatPos, vec3(0.05, 0.035, 0.045))) * 5.0;
            vec3 c0 = vec3(0.86, 0.72, 0.40), c1 = vec3(0.74, 0.46, 0.24), c2 = vec3(0.48, 0.32, 0.66);
            vec3 c3 = vec3(0.28, 0.44, 0.82), c4 = vec3(0.30, 0.64, 0.66);
            vec3 heat = t < 1.0 ? mix(c0, c1, smoothstep(0.0, 1.0, t))
                      : t < 2.0 ? mix(c1, c2, smoothstep(0.0, 1.0, t - 1.0))
                      : t < 3.0 ? mix(c2, c3, smoothstep(0.0, 1.0, t - 2.0))
                      : t < 4.0 ? mix(c3, c4, smoothstep(0.0, 1.0, t - 3.0))
                      : mix(c4, c0, smoothstep(0.0, 1.0, t - 4.0));
            diffuseColor.rgb *= mix(vec3(0.78), heat, 0.8) * 1.3;
          }`,
        );
    };
    const shell = new THREE.Mesh(new RoundedBoxGeometry(HALF * 2, HALF * 2, HALF * 2, 6, EDGE_RADIUS), shellMat);
    die.add(shell);

    const key = new THREE.DirectionalLight(0xffffff, LIGHTING.day.key);
    key.position.set(3, 6, 4);
    const ambient = new THREE.AmbientLight(0xffffff, LIGHTING.day.ambient);
    scene.add(key, ambient);

    // Screws: 4 per face, and the charging face's etching.
    const darkMat = new THREE.MeshStandardMaterial({
      ...SCREW_DARK,
      polygonOffset: true,
      polygonOffsetFactor: -2,
      polygonOffsetUnits: -2,
    });
    addScrews(die, FACE_DEFS, HALF, darkMat);
    addSeam(die, HALF, EDGE_RADIUS, darkMat);
    const etching = makeEtching(HALF, renderer.capabilities.getMaxAnisotropy(), FACE_DEFS);
    etching.mesh.material.polygonOffset = true;
    etching.mesh.material.polygonOffsetFactor = -1;
    etching.mesh.material.polygonOffsetUnits = -1;
    die.add(etching.mesh);
    let serial = "000042";
    etching.update(serial);
    // The etching font loads late; redraw once it is there.
    document.fonts?.load('600 20px "Space Grotesk"').then(() => etching.redraw());
    let glassOn = false;
    let isNight = false;
    const applyGlass = () => {
      for (const f of faces) {
        f.mesh.material = glassOn ? f.glass : f.flat;
        f.glass.emissiveIntensity = isNight ? 4.8 : 2.2;
      }
    };
    const dieX = new THREE.Vector3();
    const dieZ = new THREE.Vector3();

    const windowShape = new THREE.Shape();
    roundRectPath(windowShape, -WINDOW_HALF, -WINDOW_HALF, WINDOW_HALF * 2, WINDOW_HALF * 2, WINDOW_RADIUS);
    const windowGeo = new THREE.ShapeGeometry(windowShape, 12);
    // UVs span the full ±1 u face square, like the mockup's face canvas.
    const pos = windowGeo.attributes.position;
    const uv = windowGeo.attributes.uv;
    for (let i = 0; i < pos.count; i++) uv.setXY(i, (pos.getX(i) + 1) / 2, (pos.getY(i) + 1) / 2);

    const faces = FACE_DEFS.map((d) => {
      const out = document.createElement("canvas");
      out.width = out.height = OUT_T;
      const ctx = out.getContext("2d")!;
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, OUT_T, OUT_T);
      const panel = document.createElement("canvas");
      panel.width = panel.height = PANEL_SIZE;
      const panelCtx = panel.getContext("2d")!;
      const image = panelCtx.createImageData(PANEL_SIZE, PANEL_SIZE);
      const texture = new THREE.CanvasTexture(out);
      texture.colorSpace = THREE.SRGBColorSpace;
      texture.anisotropy = renderer.capabilities.getMaxAnisotropy();
      const flat = new THREE.MeshBasicMaterial({ map: texture, toneMapped: false });
      // Screen glass (sapphire window), design brief §4: near-black, glossy, with the screen as emissive light.
      const glass = new THREE.MeshPhysicalMaterial({
        color: 0x1c1c22,
        roughness: 0.05,
        metalness: 0,
        clearcoat: 1,
        clearcoatRoughness: 0.03,
        emissive: 0xffffff,
        emissiveMap: texture,
        envMap,
      });
      const mesh = new THREE.Mesh<THREE.BufferGeometry, THREE.Material>(windowGeo, flat);
      mesh.rotation.set(...d.rot);
      mesh.position.set(...d.n).multiplyScalar(HALF + 0.002);
      die.add(mesh);
      return { ctx, panel, panelCtx, image, texture, mesh, flat, glass };
    });

    const shadowCanvas = document.createElement("canvas");
    shadowCanvas.width = shadowCanvas.height = 128;
    const sg = shadowCanvas.getContext("2d")!;
    const grad = sg.createRadialGradient(64, 64, 4, 64, 64, 64);
    grad.addColorStop(0, "rgba(0,0,0,0.55)");
    grad.addColorStop(1, "rgba(0,0,0,0)");
    sg.fillStyle = grad;
    sg.fillRect(0, 0, 128, 128);
    const shadow = new THREE.Mesh(
      new THREE.PlaneGeometry(3.6 * K, 3.6 * K),
      new THREE.MeshBasicMaterial({ map: new THREE.CanvasTexture(shadowCanvas), transparent: true, depthWrite: false }),
    );
    shadow.rotation.x = -Math.PI / 2;
    shadow.position.y = -HALF - 0.003;
    scene.add(shadow);

    api.current = {
      drawFrames(frames) {
        frames.forEach((frame, i) => {
          const f = faces[i];
          const px = f.image.data;
          for (let b = 0; b < frame.length; b++) {
            const hi = (frame[b] >> 4) * 17;
            const lo = (frame[b] & 0x0f) * 17;
            const o = b * 8;
            px[o] = px[o + 1] = px[o + 2] = hi;
            px[o + 3] = 255;
            px[o + 4] = px[o + 5] = px[o + 6] = lo;
            px[o + 7] = 255;
          }
          f.panelCtx.putImageData(f.image, 0, 0);
          const o = f.ctx;
          o.globalCompositeOperation = "source-over";
          o.fillStyle = "#000";
          o.fillRect(0, 0, OUT_T, OUT_T);
          o.imageSmoothingEnabled = false;
          o.drawImage(f.panel, OUT_OFF, OUT_OFF, PANEL_SIZE * OUT_PX, PANEL_SIZE * OUT_PX);
          o.globalCompositeOperation = "destination-in";
          o.fillStyle = "#fff";
          o.beginPath();
          roundRectPath(o, OUT_OFF, OUT_OFF, PANEL_SIZE * OUT_PX, PANEL_SIZE * OUT_PX, MASK_RADIUS);
          o.fill();
          o.globalCompositeOperation = "source-over";
          f.texture.needsUpdate = true;
        });
      },
      setLook(finish, night) {
        const f = FINISHES.find((x) => x.key === finish) ?? FINISHES[0];
        shellMat.color.setHex(f.color);
        shellMat.metalness = f.metalness;
        shellMat.roughness = f.roughness;
        shellMat.envMapIntensity = f.envMapIntensity;
        shellMat.emissive.setHex(f.emissive ?? 0x000000);
        shellMat.emissiveIntensity = f.emissive ? (night ? GLOW_INTENSITY.night : GLOW_INTENSITY.day) : 0;
        heat.on.value = f.heatTint ? 1 : 0;
        isNight = night;
        applyGlass();
        const l = night ? LIGHTING.night : LIGHTING.day;
        renderer.toneMappingExposure = l.exposure;
        key.intensity = l.key;
        ambient.intensity = l.ambient;
      },
      setGlass(on) {
        glassOn = on;
        applyGlass();
      },
      setSerial(x) {
        if (x !== serial) {
          serial = x;
          etching.update(x);
        }
      },
      setPose({ rotation, position }) {
        die.quaternion.set(...rotation);
        die.updateMatrixWorld();
        // Slow colour drift as the die turns (heat-tinted titanium).
        dieX.setFromMatrixColumn(die.matrixWorld, 0).normalize();
        dieZ.setFromMatrixColumn(die.matrixWorld, 2).normalize();
        heat.shift.value = 0.12 * dieX.z + 0.1 * dieZ.y;
        die.position.set(...position);
        shadow.scale.setScalar(1 - Math.min(0.35, position[1] * 0.25));
        (shadow.material as THREE.MeshBasicMaterial).opacity = 1 - Math.min(0.6, position[1] * 0.4);
        shadow.position.x = position[0];
      },
      viewerRight() {
        const forward = new THREE.Vector3().subVectors(look, camera.position);
        forward.y = 0;
        forward.normalize();
        const right = new THREE.Vector3().crossVectors(forward, new THREE.Vector3(0, 1, 0)).normalize();
        return [right.x, right.y, right.z];
      },
    };

    // Input, as in the mockup: press a face to touch it; moving more than
    // 8 px turns the press into a drag that turns the die in the hand.
    const raycaster = new THREE.Raycaster();
    let press: {
      x: number;
      y: number;
      lastX: number;
      lastY: number;
      face: number | null;
      dragging: boolean;
      swiped: boolean;
      /** Set once a multi-turn drag has picked its axis. */
      spin: SpinAxis | null;
    } | null = null;
    // Holding F makes a menu drag a multi-turn (see `multiTurn`). A plain
    // key rather than a modifier: Ctrl-click is a right-click on a Mac.
    let fHeld = false;
    const setF = (held: boolean) => {
      if (held !== fHeld) {
        fHeld = held;
        cbs.current.onMultiKey(held);
      }
    };
    // Match the physical key, so it works whatever the keyboard layout or
    // modifiers; ignore it only while typing into a text field.
    const isF = (e: KeyboardEvent) => e.code === "KeyF" || e.key === "f" || e.key === "F";
    const typing = (e: KeyboardEvent) => {
      const t = e.target;
      if (t instanceof HTMLTextAreaElement || (t instanceof HTMLElement && t.isContentEditable)) return true;
      return t instanceof HTMLInputElement && !["checkbox", "radio", "button", "range"].includes(t.type);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (isF(e) && !typing(e)) setF(true);
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (isF(e)) setF(false);
    };
    const onBlur = () => setF(false);
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    const faceAt = (e: PointerEvent): number | null => {
      const rect = renderer.domElement.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1,
      );
      raycaster.setFromCamera(ndc, camera);
      const hit = raycaster.intersectObjects(
        faces.map((f) => f.mesh),
        false,
      )[0];
      return hit ? faces.findIndex((f) => f.mesh === hit.object) : null;
    };
    const onDown = (e: PointerEvent) => {
      renderer.domElement.setPointerCapture(e.pointerId);
      const face = faceAt(e);
      press = {
        x: e.clientX,
        y: e.clientY,
        lastX: e.clientX,
        lastY: e.clientY,
        face,
        dragging: false,
        swiped: false,
        spin: null,
      };
      if (face !== null) cbs.current.onTouch(face, true);
    };
    const onMove = (e: PointerEvent) => {
      if (!press) return;
      if (!press.dragging && Math.hypot(e.clientX - press.x, e.clientY - press.y) > 8) {
        press.dragging = true;
        if (press.face !== null) cbs.current.onTouch(press.face, false);
        press.face = null;
      }
      const dx = e.clientX - press.x;
      const dy = e.clientY - press.y;
      const multi = cbs.current.multiTurn || fHeld;
      if (cbs.current.swipeToTip && press.dragging && !press.swiped && !press.spin && multi) {
        // Multi-turn: the drag's dominant direction picks the axis.
        press.spin = Math.abs(dx) > Math.abs(dy) ? "yaw" : "pitch";
      }
      if (press.spin) {
        cbs.current.onSpin(press.spin, (press.spin === "yaw" ? dx : dy) * SPIN_RAD_PER_PX);
      } else if (cbs.current.swipeToTip) {
        // One tip per swipe, by dominant direction.
        if (!press.swiped && Math.hypot(dx, dy) > SWIPE_PX) {
          press.swiped = true;
          cbs.current.onTip(Math.abs(dx) > Math.abs(dy) ? (dx > 0 ? "right" : "left") : dy < 0 ? "up" : "down");
        }
      } else if (press.dragging) {
        cbs.current.onRotate((e.clientX - press.lastX) * 0.008, (e.clientY - press.lastY) * 0.008);
      }
      press.lastX = e.clientX;
      press.lastY = e.clientY;
    };
    // Touch browsers may cancel a pointer mid-gesture; treat it as a release.
    const onUp = () => {
      if (press?.face != null) cbs.current.onTouch(press.face, false);
      if (press?.spin) cbs.current.onSpinEnd();
      press = null;
    };
    const noMenu = (e: Event) => e.preventDefault();
    renderer.domElement.addEventListener("pointerdown", onDown);
    renderer.domElement.addEventListener("pointermove", onMove);
    renderer.domElement.addEventListener("pointerup", onUp);
    renderer.domElement.addEventListener("pointercancel", onUp);
    renderer.domElement.addEventListener("contextmenu", noMenu);

    // Camera (SIM_SPEC A5): 32° FOV along (0.55, 0.62, 1), farther in portrait.
    const resize = () => {
      const { clientWidth: w, clientHeight: h } = mount;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      const portrait = camera.aspect < 0.75;
      const dist = (portrait ? 12.5 : 9.5) * K;
      look.set(0, (portrait ? -0.1 : -0.35) * K, 0);
      camera.position.set(0.55, 0.62, 1).normalize().multiplyScalar(dist);
      camera.lookAt(look);
      camera.updateProjectionMatrix();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(mount);
    resize();

    let raf = 0;
    const loop = () => {
      raf = requestAnimationFrame(loop);
      renderer.render(scene, camera);
    };
    loop();

    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      cancelAnimationFrame(raf);
      observer.disconnect();
      renderer.dispose();
      pmrem.dispose();
      etching.mesh.geometry.dispose();
      faces.forEach((f) => {
        f.texture.dispose();
        f.glass.dispose();
      });
      mount.removeChild(renderer.domElement);
    };
  }, []);

  return <div className="die-view" ref={mountRef} />;
});
