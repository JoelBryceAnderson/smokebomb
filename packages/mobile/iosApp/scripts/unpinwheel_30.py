#!/usr/bin/env python3
"""Turn the 30 mm x-ray's face modules out of the pinwheel, and re-run the harness.

Until 4 Oct the 30 mm die's six display modules were pinwheeled: each
screen's ribbon left a different way round the cube. Now (sugarcube-
assembly.html, `FACES`) the four side screens' ribbons point down, toward
the lid; the top screen's points to +X, down the harness channel; the lid
screen's to -Z. The firmware turns each picture to suit (`Rgb64::MOUNT`).

The exported x-rays still have the pinwheel. This script fixes them in place
of a re-export:

1. Each face's module (the triangles of `Internal_panel_glass`, `_encap`,
   `_chip` and `_fpc` nearest that face) turns about the face's axis, from
   its old ribbon direction to its new one: +X a half turn, +Y, +Z and -Z a
   quarter turn; -X and the lid's were already right. Points and normals
   turn; nothing else changes.
2. The screen harness (`Internal_w_spi_a`, `_b`, `_c`) is rebuilt: from each
   cup screen's ribbon end in to the wall, round a ring just above the board
   (clear of the corner pillars, under the cell) to the +X side and the
   board's connector. Each wire keeps its prim, colour and material.

Exploded x-rays keep their modules where they are, out along each face; the
harness is drawn inside, where it is in the assembled die.

The packages are written as they are (the edited root layer saved in place,
its textures beside it), not flattened again, so check them rather than
converting them:

Usage:   python3 unpinwheel_30.py ../iosApp/Resources/Models/sugarcube_30_xray.usdz \\
             ../iosApp/Resources/Models/sugarcube_30_xray_exploded.usdz --out /tmp/unpinwheeled
         python3 convert_usdz.py --check-only /tmp/unpinwheeled/*.usdz
         cp /tmp/unpinwheeled/*.usdz ../iosApp/Resources/Models/
"""

import argparse
import math
import os
import tempfile
import zipfile

from pxr import Gf, Usd, UsdGeom, UsdUtils, Vt

MM = 0.001
HALF = 15 * MM  # the 30 mm die

# Old ribbon direction -> new, per face (outward normal): the turn about that axis.
TURNS = {
    "px": ((1, 0, 0), 180),
    "py": ((0, 1, 0), 90),   # +Z -> +X
    "pz": ((0, 0, 1), -90),  # +X -> -Y
    "nz": ((0, 0, 1), 90),   # -X -> -Y
}
# The new ribbon directions, for finding each ribbon's end.
RIBBON = {
    "px": (0, -1, 0), "nx": (0, -1, 0), "pz": (0, -1, 0), "nz": (0, -1, 0),
    "py": (1, 0, 0), "ny": (0, 0, -1),
}
NORMAL = {
    "px": (1, 0, 0), "nx": (-1, 0, 0), "py": (0, 1, 0),
    "ny": (0, -1, 0), "pz": (0, 0, 1), "nz": (0, 0, -1),
}
MODULE_PARTS = ["Internal_panel_glass", "Internal_encap", "Internal_chip", "Internal_fpc"]
# Which wire mesh carries which screens.
WIRES = {"Internal_w_spi_a": ["px", "py"], "Internal_w_spi_b": ["nx", "pz"], "Internal_w_spi_c": ["nz"]}

# The harness, in the assembled die's mm (y up from the lid face).
LANE = 11.4          # inside the frame, outside the cell and the board
RING_Y = 6.0         # above the board, its parts and the pillars; under the cell (7.85)
RING_STEP = 0.35     # each wire's own height round the ring
HUB = (9.75, 5.35)   # the board's screen connector (x, y), on its +X edge
HUB_Z = [-0.4, 0.0, 0.4, 0.8, 1.2]
WIRE_R = 0.3
SIDES = 8


def face_of(p, centre):
    d = p - centre
    a = [abs(d[0]), abs(d[1]), abs(d[2])]
    i = a.index(max(a))
    return ["px", "py", "pz"][i] if d[i] > 0 else ["nx", "ny", "nz"][i]


def rotation(axis, degrees):
    return Gf.Matrix4d().SetRotate(Gf.Rotation(Gf.Vec3d(*axis), degrees))


def find(stage, name):
    for p in stage.Traverse():
        if p.GetName() == name:
            return p
    raise SystemExit(f"no {name} in the model")


def die_base(stage):
    cache = UsdGeom.BBoxCache(Usd.TimeCode.Default(), ["default", "render"])
    return cache.ComputeWorldBound(find(stage, "Shell")).ComputeAlignedRange().GetMin()[1]


def turn_modules(stage, centre):
    """Turns each face's module about its face's axis; returns the turned
    `Internal_fpc` points per face."""
    ribbons = {}
    for name in MODULE_PARTS:
        mesh = UsdGeom.Mesh(find(stage, name))
        if mesh.GetPrim().GetAttribute("xformOpOrder").Get():
            raise SystemExit(f"{name} has a transform of its own; this script expects points in the die's frame")
        points = list(mesh.GetPointsAttr().Get())
        normals_attr = mesh.GetNormalsAttr()
        normals = list(normals_attr.Get() or [])
        if normals and mesh.GetNormalsInterpolation() != UsdGeom.Tokens.vertex:
            raise SystemExit(f"{name}'s normals aren't per vertex")
        counts = mesh.GetFaceVertexCountsAttr().Get()
        indices = mesh.GetFaceVertexIndicesAttr().Get()
        face_of_point = {}
        k = 0
        for c in counts:
            ids = indices[k:k + c]
            k += c
            mid = sum((Gf.Vec3d(points[i]) for i in ids), Gf.Vec3d(0, 0, 0)) / c
            f = face_of(mid, centre)
            for i in ids:
                if face_of_point.setdefault(i, f) != f:
                    raise SystemExit(f"{name}: a point is shared by two faces' modules")
        for i, f in face_of_point.items():
            if f not in TURNS:
                continue
            axis, degrees = TURNS[f]
            m = rotation(axis, degrees)
            points[i] = Gf.Vec3f(m.Transform(Gf.Vec3d(points[i]) - centre) + centre)
            if normals:
                normals[i] = Gf.Vec3f(m.TransformDir(Gf.Vec3d(normals[i])))
        mesh.GetPointsAttr().Set(Vt.Vec3fArray(points))
        if normals:
            normals_attr.Set(Vt.Vec3fArray(normals))
        mesh.GetExtentAttr().Set(UsdGeom.PointBased.ComputeExtent(Vt.Vec3fArray(points)))
        if name == "Internal_fpc":
            for i, f in face_of_point.items():
                ribbons.setdefault(f, []).append(Gf.Vec3d(points[i]))
    return ribbons


def ribbon_ends(ribbons, centre):
    """Where each ribbon ends, a little inside it, in the die's mm relative to
    the die's centre (so the same path serves an exploded model)."""
    ends = {}
    for f, pts in ribbons.items():
        u, n = Gf.Vec3d(*RIBBON[f]), Gf.Vec3d(*NORMAL[f])
        along = [p * u for p in pts]
        top = max(along)
        end = [p for p in pts if p * u > top - 0.6 * MM]
        mid = sum(end, Gf.Vec3d(0, 0, 0)) / len(end)
        inside = min(p * n for p in pts) - 0.4 * MM
        mid = mid - n * (mid * n) + n * inside
        ends[f] = (mid - centre) / MM
    return ends


def path(face, end, k):
    """A cup screen's harness wire, mm relative to the die's centre (y from -15)."""
    y0 = -15.0
    ring = y0 + RING_Y + RING_STEP * k
    hub_y = y0 + HUB[1]
    z_hub = HUB_Z[k]
    e = end
    pts = [Gf.Vec3d(e)]
    if face == "px":
        pts += [Gf.Vec3d(LANE, e[1], e[2]), Gf.Vec3d(LANE, ring, e[2]), Gf.Vec3d(LANE, ring, z_hub)]
    elif face == "py":
        pts += [Gf.Vec3d(LANE, e[1], e[2]), Gf.Vec3d(LANE, ring, e[2]), Gf.Vec3d(LANE, ring, z_hub)]
    elif face == "nx":
        pts += [Gf.Vec3d(-LANE, e[1], e[2]), Gf.Vec3d(-LANE, ring, e[2]), Gf.Vec3d(-LANE, ring, LANE),
                Gf.Vec3d(LANE, ring, LANE), Gf.Vec3d(LANE, ring, z_hub)]
    elif face == "pz":
        pts += [Gf.Vec3d(e[0], e[1], LANE), Gf.Vec3d(e[0], ring, LANE), Gf.Vec3d(LANE, ring, LANE),
                Gf.Vec3d(LANE, ring, z_hub)]
    elif face == "nz":
        pts += [Gf.Vec3d(e[0], e[1], -LANE), Gf.Vec3d(e[0], ring, -LANE), Gf.Vec3d(LANE, ring, -LANE),
                Gf.Vec3d(LANE, ring, z_hub)]
    pts += [Gf.Vec3d(HUB[0] + 0.6, ring, z_hub), Gf.Vec3d(HUB[0], hub_y, z_hub)]
    out = [pts[0]]
    for p in pts[1:]:
        if (p - out[-1]).GetLength() > 1e-3:
            out.append(p)
    return out


def tube(paths):
    """Round tubes along polylines (mm): points (m), normals, st, counts, indices.
    A ring of points at every corner, square to the turn's bisector."""
    points, normals, st, counts, indices = [], [], [], [], []
    for pts in paths:
        base = len(points)
        length = 0.0
        side = Gf.Vec3d(0, 1, 0)
        for i, p in enumerate(pts):
            a = (pts[i] - pts[i - 1]) if i > 0 else (pts[1] - pts[0])
            b = (pts[i + 1] - pts[i]) if i + 1 < len(pts) else a
            t = a.GetNormalized() + b.GetNormalized()
            t = t.GetNormalized() if t.GetLength() > 1e-6 else b.GetNormalized()
            if i > 0:
                length += (pts[i] - pts[i - 1]).GetLength()
            side = side - t * (side * t)
            if side.GetLength() < 1e-6:
                side = Gf.Vec3d(1, 0, 0) - t * t[0]
            side = side.GetNormalized()
            other = Gf.Cross(t, side).GetNormalized()
            for k in range(SIDES):
                ang = 2 * math.pi * k / SIDES
                d = side * math.cos(ang) + other * math.sin(ang)
                points.append(Gf.Vec3f((p + d * WIRE_R) * MM))
                normals.append(Gf.Vec3f(d))
                st.append(Gf.Vec2f(length / 10.0, k / SIDES))
        for i in range(len(pts) - 1):
            for k in range(SIDES):
                a0 = base + i * SIDES + k
                a1 = base + i * SIDES + (k + 1) % SIDES
                counts.append(4)
                indices += [a0, a1, a1 + SIDES, a0 + SIDES]
    return points, normals, st, counts, indices


def rebuild_harness(stage, ends, centre):
    k = 0
    for name, faces in WIRES.items():
        mesh = UsdGeom.Mesh(find(stage, name))
        paths = []
        for f in faces:
            paths.append([q * MM + centre for q in path(f, ends[f], k)])
            k += 1
        # Back to mm for tube(): it scales to metres itself.
        points, normals, st, counts, indices = tube([[q / MM for q in p] for p in paths])
        mesh.GetPointsAttr().Set(Vt.Vec3fArray(points))
        mesh.GetNormalsAttr().Set(Vt.Vec3fArray(normals))
        mesh.SetNormalsInterpolation(UsdGeom.Tokens.vertex)
        mesh.GetFaceVertexCountsAttr().Set(Vt.IntArray(counts))
        mesh.GetFaceVertexIndicesAttr().Set(Vt.IntArray(indices))
        st_var = UsdGeom.PrimvarsAPI(mesh).GetPrimvar("st")
        if st_var:
            st_var.Set(Vt.Vec2fArray(st))
            st_var.SetInterpolation(UsdGeom.Tokens.vertex)
        mesh.GetExtentAttr().Set(UsdGeom.PointBased.ComputeExtent(Vt.Vec3fArray(points)))


def process(src, dst, harness_ends=None):
    with tempfile.TemporaryDirectory() as tmp:
        tmp = os.path.realpath(tmp)
        with zipfile.ZipFile(src) as z:
            names = z.namelist()
            z.extractall(tmp)
        stage = Usd.Stage.Open(os.path.join(tmp, names[0]))
        base = die_base(stage)
        centre = Gf.Vec3d(0, base + HALF, 0)
        ribbons = turn_modules(stage, centre)
        ends = harness_ends or ribbon_ends(ribbons, centre)
        rebuild_harness(stage, ends, centre)
        # Saved in place, so textures keep their paths relative to the root.
        root = stage.GetRootLayer()
        if not root.Save():
            raise SystemExit(f"couldn't save {names[0]}")
        if os.path.exists(dst):
            os.remove(dst)
        if not UsdUtils.CreateNewARKitUsdzPackage(root.realPath, dst):
            raise SystemExit(f"couldn't package {dst}")
    return ends


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("inputs", nargs="+", help="the 30 mm x-ray usdz files (assembled first)")
    parser.add_argument("--out", required=True, help="output folder; check what's there with convert_usdz.py --check-only")
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    # The assembled x-ray sets where the harness runs; an exploded one reuses it, inside the die.
    assembled = [p for p in args.inputs if "exploded" not in p]
    exploded = [p for p in args.inputs if "exploded" in p]
    if exploded and not assembled:
        raise SystemExit("give the assembled x-ray too: it sets where the harness runs")
    ends = None
    for src in assembled + exploded:
        dst = os.path.join(args.out, os.path.basename(src))
        ends = process(src, dst, ends if src in exploded else None)
        print(f"{os.path.basename(src)}: modules turned, harness rebuilt -> {dst}")


if __name__ == "__main__":
    main()
