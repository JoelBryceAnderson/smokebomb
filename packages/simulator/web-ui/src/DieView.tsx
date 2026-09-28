import { forwardRef, useEffect, useImperativeHandle, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import { FACE_COUNT, PANEL_SIZE } from "./protocol";

/** Outward normal of each face in the die's body frame (BoxGeometry order). */
export const FACE_NORMALS = [
  new THREE.Vector3(1, 0, 0),
  new THREE.Vector3(-1, 0, 0),
  new THREE.Vector3(0, 1, 0),
  new THREE.Vector3(0, -1, 0),
  new THREE.Vector3(0, 0, 1),
  new THREE.Vector3(0, 0, -1),
];

export interface DieViewHandle {
  /** Paint six packed 4bpp frames onto the cube faces. */
  drawFrames(faces: Uint8Array[]): void;
  /** Animate the cube to a new orientation (body -> world). */
  setOrientation(q: THREE.Quaternion, tumble?: boolean): void;
}

interface Props {
  onTouch(face: number, pressed: boolean): void;
}

// OLED phosphor tint for full-brightness pixels.
const TINT = [236, 244, 255];

export const DieView = forwardRef<DieViewHandle, Props>(function DieView({ onTouch }, ref) {
  const mountRef = useRef<HTMLDivElement>(null);
  const api = useRef<DieViewHandle | null>(null);
  const touchCb = useRef(onTouch);
  touchCb.current = onTouch;

  useImperativeHandle(ref, () => ({
    drawFrames: (f) => api.current?.drawFrames(f),
    setOrientation: (q, t) => api.current?.setOrientation(q, t),
  }));

  useEffect(() => {
    const mount = mountRef.current!;
    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(window.devicePixelRatio);
    mount.appendChild(renderer.domElement);

    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0x0d0d10);
    const camera = new THREE.PerspectiveCamera(35, 1, 0.1, 100);
    camera.position.set(3.2, 2.6, 3.8);

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enablePan = false;
    controls.minDistance = 3;
    controls.maxDistance = 9;

    // One canvas texture per OLED panel.
    const panels = Array.from({ length: FACE_COUNT }, () => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = PANEL_SIZE;
      const ctx = canvas.getContext("2d")!;
      const image = ctx.createImageData(PANEL_SIZE, PANEL_SIZE);
      const texture = new THREE.CanvasTexture(canvas);
      texture.magFilter = THREE.NearestFilter;
      texture.colorSpace = THREE.SRGBColorSpace;
      return { ctx, image, texture };
    });
    const materials = panels.map((p) => new THREE.MeshBasicMaterial({ map: p.texture }));
    const die = new THREE.Mesh(new THREE.BoxGeometry(1.6, 1.6, 1.6), materials);
    const bezel = new THREE.LineSegments(
      new THREE.EdgesGeometry(new THREE.BoxGeometry(1.62, 1.62, 1.62)),
      new THREE.LineBasicMaterial({ color: 0x3a3a44 }),
    );
    die.add(bezel);
    scene.add(die);

    let target = die.quaternion.clone();
    let tumbleUntil = 0;

    api.current = {
      drawFrames(faces) {
        faces.forEach((frame, i) => {
          const { ctx, image, texture } = panels[i];
          const px = image.data;
          for (let b = 0; b < frame.length; b++) {
            for (let n = 0; n < 2; n++) {
              const level = (n === 0 ? frame[b] >> 4 : frame[b] & 0x0f) / 15;
              const o = (b * 2 + n) * 4;
              px[o] = TINT[0] * level;
              px[o + 1] = TINT[1] * level;
              px[o + 2] = TINT[2] * level;
              px[o + 3] = 255;
            }
          }
          ctx.putImageData(image, 0, 0);
          texture.needsUpdate = true;
        });
      },
      setOrientation(q, tumble = false) {
        target = q.clone();
        tumbleUntil = tumble ? performance.now() + 900 : 0;
      },
    };

    // Touch: pointer down on a face presses it; dragging elsewhere orbits.
    const raycaster = new THREE.Raycaster();
    let pressed: number | null = null;
    const faceAt = (e: PointerEvent): number | null => {
      const rect = renderer.domElement.getBoundingClientRect();
      const ndc = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1,
      );
      raycaster.setFromCamera(ndc, camera);
      const hit = raycaster.intersectObject(die, false)[0];
      return hit?.face ? hit.face.materialIndex : null;
    };
    const onDown = (e: PointerEvent) => {
      const face = faceAt(e);
      if (face === null) return;
      controls.enabled = false;
      pressed = face;
      touchCb.current(face, true);
    };
    const onUp = () => {
      if (pressed !== null) touchCb.current(pressed, false);
      pressed = null;
      controls.enabled = true;
    };
    // Capture phase so we run before OrbitControls sees the event.
    renderer.domElement.addEventListener("pointerdown", onDown, { capture: true });
    window.addEventListener("pointerup", onUp);

    const resize = () => {
      const { clientWidth: w, clientHeight: h } = mount;
      renderer.setSize(w, h);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(mount);
    resize();

    let frame = 0;
    const loop = () => {
      frame = requestAnimationFrame(loop);
      const now = performance.now();
      if (now < tumbleUntil) {
        die.rotateOnAxis(new THREE.Vector3(1, 0.6, 0.3).normalize(), 0.35);
      } else {
        die.quaternion.slerp(target, 0.18);
      }
      controls.update();
      renderer.render(scene, camera);
    };
    loop();

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("pointerup", onUp);
      controls.dispose();
      renderer.dispose();
      panels.forEach((p) => p.texture.dispose());
      mount.removeChild(renderer.domElement);
    };
  }, []);

  return <div className="die-view" ref={mountRef} />;
});
