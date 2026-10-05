#!/usr/bin/env python3
"""Write a stand-in 30 mm die that follows the per-part naming contract.

The real per-part models aren't exported yet. This one is crude boxes, but its
prims are named exactly as AR_VIEWER.md's contract says (Shell, Lid,
Window_<face>, Module_<face>, Screw_0..3, Pillar_0..3, Board, Cell,
BalancePlate, Wiring), so explode, the x-ray fade and the lid can be tried
on a phone today. Like the hand-written models, its root layer is text USD;
run it through convert_usdz.py before bundling.

    python3 make_contract_fixture.py out/sugarcube_contract_fixture.usdz
"""

import argparse
import os
import tempfile

from pxr import Gf, Sdf, Usd, UsdGeom, UsdShade, UsdUtils

SIZE = 0.030  # metres, like sugarcube_30
HALF = SIZE / 2
CENTRE = Gf.Vec3f(0, HALF, 0)  # origin at the bottom centre, Y up
WALL = 0.0015
OPENING = 0.022

# (name, axis index, sign); ny is the lid.
FACES = [("px", 0, 1), ("nx", 0, -1), ("py", 1, 1), ("ny", 1, -1), ("pz", 2, 1), ("nz", 2, -1)]
SCREEN_COLOURS = {
    "px": (0.95, 0.35, 0.30),
    "nx": (0.30, 0.80, 0.45),
    "py": (0.35, 0.55, 0.95),
    "ny": (0.90, 0.85, 0.35),
    "pz": (0.80, 0.40, 0.90),
    "nz": (0.35, 0.85, 0.90),
}


def axes_for(axis):
    """The two in-plane axes of a face whose normal is along `axis`."""
    return [a for a in range(3) if a != axis]


def box(stage, path, centre, size, material):
    """An axis-aligned box mesh with flat normals."""
    mesh = UsdGeom.Mesh.Define(stage, path)
    cx, cy, cz = centre
    sx, sy, sz = (s / 2 for s in size)
    corners = [Gf.Vec3f(cx + dx * sx, cy + dy * sy, cz + dz * sz) for dx in (-1, 1) for dy in (-1, 1) for dz in (-1, 1)]
    # Corner index = (dx>0)*4 + (dy>0)*2 + (dz>0); each quad wound counter-clockwise from outside.
    quads = [
        ((4, 6, 7, 5), (1, 0, 0)),
        ((0, 1, 3, 2), (-1, 0, 0)),
        ((2, 3, 7, 6), (0, 1, 0)),
        ((0, 4, 5, 1), (0, -1, 0)),
        ((1, 5, 7, 3), (0, 0, 1)),
        ((0, 2, 6, 4), (0, 0, -1)),
    ]
    points, normals, indices = [], [], []
    for quad, n in quads:
        for i in quad:
            indices.append(len(points))
            points.append(corners[i])
            normals.append(Gf.Vec3f(*n))
    mesh.CreatePointsAttr(points)
    mesh.CreateFaceVertexCountsAttr([4] * 6)
    mesh.CreateFaceVertexIndicesAttr(indices)
    mesh.CreateNormalsAttr(normals)
    mesh.SetNormalsInterpolation(UsdGeom.Tokens.faceVarying)
    mesh.CreateSubdivisionSchemeAttr(UsdGeom.Tokens.none)
    mesh.CreateExtentAttr([Gf.Vec3f(cx - sx, cy - sy, cz - sz), Gf.Vec3f(cx + sx, cy + sy, cz + sz)])
    UsdShade.MaterialBindingAPI.Apply(mesh.GetPrim()).Bind(material)
    return mesh


def material(stage, name, colour, metallic=0.0, roughness=0.5, opacity=1.0, emissive=None):
    mat = UsdShade.Material.Define(stage, f"/Sugarcube/Materials/{name}")
    shader = UsdShade.Shader.Define(stage, f"/Sugarcube/Materials/{name}/Surface")
    shader.CreateIdAttr("UsdPreviewSurface")
    shader.CreateInput("diffuseColor", Sdf.ValueTypeNames.Color3f).Set(Gf.Vec3f(*colour))
    shader.CreateInput("metallic", Sdf.ValueTypeNames.Float).Set(metallic)
    shader.CreateInput("roughness", Sdf.ValueTypeNames.Float).Set(roughness)
    if opacity < 1:
        shader.CreateInput("opacity", Sdf.ValueTypeNames.Float).Set(opacity)
    if emissive:
        shader.CreateInput("emissiveColor", Sdf.ValueTypeNames.Color3f).Set(Gf.Vec3f(*emissive))
    mat.CreateSurfaceOutput().ConnectToSource(shader.ConnectableAPI(), "surface")
    return mat


def offset(axis, distance):
    v = [0.0, 0.0, 0.0]
    v[axis] = distance
    return Gf.Vec3f(*v)


def build(stage):
    UsdGeom.SetStageUpAxis(stage, UsdGeom.Tokens.y)
    UsdGeom.SetStageMetersPerUnit(stage, 1.0)
    root = UsdGeom.Xform.Define(stage, "/Sugarcube")
    stage.SetDefaultPrim(root.GetPrim())
    Usd.ModelAPI(root).SetKind("component")
    UsdGeom.Scope.Define(stage, "/Sugarcube/Materials")

    titanium = material(stage, "Titanium", (0.62, 0.62, 0.64), metallic=1.0, roughness=0.35)
    sapphire = material(stage, "Sapphire", (0.85, 0.9, 1.0), roughness=0.05, opacity=0.25)
    panel = material(stage, "PanelGlass", (0.05, 0.05, 0.06), roughness=0.2)
    chip = material(stage, "Driver", (0.1, 0.1, 0.1), roughness=0.6)
    ribbon = material(stage, "Ribbon", (0.85, 0.55, 0.15), roughness=0.5)
    pcb = material(stage, "Board", (0.1, 0.45, 0.2), roughness=0.6)
    cell = material(stage, "Cell", (0.75, 0.75, 0.78), metallic=1.0, roughness=0.3)
    tungsten = material(stage, "Tungsten", (0.3, 0.3, 0.32), metallic=1.0, roughness=0.4)
    wire = material(stage, "Wire", (0.85, 0.15, 0.15), roughness=0.5)
    screens = {f: material(stage, f"Screen_{f}", (0, 0, 0), emissive=c) for f, c in SCREEN_COLOURS.items()}

    UsdGeom.Xform.Define(stage, "/Sugarcube/Shell")
    UsdGeom.Xform.Define(stage, "/Sugarcube/Lid")
    frame_width = (SIZE - OPENING) / 2
    for face, axis, sign in FACES:
        u, v = axes_for(axis)
        normal_offset = sign * (HALF - WALL / 2)
        parent = "/Sugarcube/Lid" if face == "ny" else "/Sugarcube/Shell"
        # The frame around the opening: two full-width strips and two short ones.
        for i, (du, dv, su, sv) in enumerate(
            [
                (HALF - frame_width / 2, 0, frame_width, SIZE),
                (-(HALF - frame_width / 2), 0, frame_width, SIZE),
                (0, HALF - frame_width / 2, OPENING, frame_width),
                (0, -(HALF - frame_width / 2), OPENING, frame_width),
            ]
        ):
            c = CENTRE + offset(axis, normal_offset) + offset(u, du) + offset(v, dv)
            size = [0.0, 0.0, 0.0]
            size[axis], size[u], size[v] = WALL, su, sv
            box(stage, f"{parent}/Frame_{face}_{i}", c, size, titanium)

        # Window: sapphire filling the opening, flush with the outside.
        size = [0.0, 0.0, 0.0]
        size[axis], size[u], size[v] = WALL, OPENING, OPENING
        UsdGeom.Xform.Define(stage, f"/Sugarcube/Window_{face}")
        box(stage, f"/Sugarcube/Window_{face}/Sapphire", CENTRE + offset(axis, normal_offset), size, sapphire)

        # Module: panel glass, screen, driver chip and ribbon, just inside the window.
        module = f"/Sugarcube/Module_{face}"
        UsdGeom.Xform.Define(stage, module)
        depth = HALF - WALL - 0.0006
        size = [0.0, 0.0, 0.0]
        size[axis], size[u], size[v] = 0.0012, 0.020, 0.020
        box(stage, f"{module}/Glass", CENTRE + offset(axis, sign * depth), size, panel)
        size[axis] = 0.0002
        size[u] = size[v] = 0.017
        box(stage, f"{module}/Screen_{face}", CENTRE + offset(axis, sign * (depth + 0.0007)), size, screens[face])
        size[axis], size[u], size[v] = 0.0006, 0.004, 0.002
        box(stage, f"{module}/Driver", CENTRE + offset(axis, sign * (depth - 0.0009)) + offset(u, -0.006), size, chip)
        size[axis], size[u], size[v] = 0.0002, 0.006, 0.008
        box(stage, f"{module}/Ribbon", CENTRE + offset(axis, sign * (depth - 0.0008)) + offset(v, 0.006), size, ribbon)

    corner = HALF - 0.0035
    for i, (x, z) in enumerate([(corner, corner), (-corner, corner), (-corner, -corner), (corner, -corner)]):
        UsdGeom.Xform.Define(stage, f"/Sugarcube/Screw_{i}")
        box(stage, f"/Sugarcube/Screw_{i}/Head", Gf.Vec3f(x, WALL / 2, z), [0.002, WALL, 0.002], titanium)
        UsdGeom.Xform.Define(stage, f"/Sugarcube/Pillar_{i}")
        box(stage, f"/Sugarcube/Pillar_{i}/Post", Gf.Vec3f(x, HALF, z), [0.0025, SIZE - 2 * WALL, 0.0025], titanium)

    UsdGeom.Xform.Define(stage, "/Sugarcube/BalancePlate")
    box(stage, "/Sugarcube/BalancePlate/Plate", Gf.Vec3f(0, 0.0045, 0), [0.016, 0.0015, 0.016], tungsten)
    UsdGeom.Xform.Define(stage, "/Sugarcube/Cell")
    box(stage, "/Sugarcube/Cell/Body", Gf.Vec3f(0, 0.0095, 0), [0.016, 0.006, 0.012], cell)
    UsdGeom.Xform.Define(stage, "/Sugarcube/Board")
    box(stage, "/Sugarcube/Board/PCB", Gf.Vec3f(0, 0.0145, 0), [0.019, 0.0012, 0.019], pcb)
    UsdGeom.Xform.Define(stage, "/Sugarcube/Wiring")
    for i, (x, z) in enumerate([(0.008, 0.0), (-0.008, 0.0), (0.0, 0.008), (0.0, -0.008)]):
        box(stage, f"/Sugarcube/Wiring/Wire_{i}", Gf.Vec3f(x, 0.019, z), [0.0006, 0.008, 0.0006], wire)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", help="the .usdz to write (text USD root, like the hand-written models)")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory() as tmp:
        usda = os.path.join(tmp, "sugarcube_contract_fixture.usda")
        stage = Usd.Stage.CreateNew(usda)
        build(stage)
        stage.GetRootLayer().Save()
        os.makedirs(os.path.dirname(os.path.abspath(args.output)), exist_ok=True)
        if not UsdUtils.CreateNewUsdzPackage(usda, args.output):
            raise SystemExit("couldn't write the package")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
