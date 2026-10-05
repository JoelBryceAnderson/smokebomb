#!/usr/bin/env python3
"""Bake the charging face's laser etching into the Sugarcube models' lids.

The etching is what the desktop simulator draws on the charging face
(`drawEtching` in packages/simulator/web-ui/src/shell.ts): the wordmark and
the tagline in the script, the model and serial, and CE, the crossed-out bin
and the regulatory line, round the window on the lid (the -Y face).

For every `Shell` mesh in a model (one per die; the line-up has three), the
flat faces of the lid round the window move into a mesh of their own named
`Etching`, beside the shell. Its points and normals are the shell's own, so
nothing moves and the true size is as exported. It gets UVs across the flat
face and a copy of the shell's material with the look baked in at 2048 px:

- colour: the shell's own (a tinted titanium's texture too), lifted 55 %
  toward the simulator's grey ink where it's marked
- roughness and metallic: the shell's, to 0.7 and 0.3 where it's marked, as
  the simulator's decal
- normal: the shell's brushed grain, plus the marking's edges, so they catch
  the light like a cut
- anything else the shell sets, such as an x-ray's opacity, carries over

The output keeps the input's packaging (a text root), so run it before
convert_usdz.py:

    python3 etch_lids.py ~/Downloads/sugarcube_*.usdz --out /tmp/etched
    python3 convert_usdz.py /tmp/etched/*.usdz

A model that's already etched, or has no die shell (the contract fixture),
is copied unchanged.
"""

import argparse
import os
import shutil
import struct
import sys
import tempfile
import zipfile

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from pxr import Gf, Sdf, Usd, UsdGeom, UsdShade, Vt

HERE = os.path.dirname(os.path.abspath(__file__))
FONTS = os.path.normpath(os.path.join(HERE, "..", "..", "..", "firmware", "assets", "fonts"))
SCRIPT_FONT = os.path.join(FONTS, "pacifico-latin-400-normal.woff")
# The simulator asks for Space Grotesk 600; the firmware ships the 700.
PLAIN_FONT = os.path.join(FONTS, "space-grotesk-latin-700-normal.woff")

WORDMARK = "Sugarcube"
TAGLINE = "Designed in Williamsburg, BK · Shake well before serving"
REGULATORY = "REGULATORY INFO IN SETTINGS"
# Room for the bin between CE and the regulatory line.
GAP = "      "
MODEL = "SC-1"
# The serial the simulator etches before a roll carries the die's own.
SERIAL = "000042"

# Per die size (mm): the flat face, the window's half-size, the band's
# centre line, and whether the script lines are centred in the band (true) or
# hug the window. 30 and 34 are the simulator's (`GEOMETRY` in
# web-ui/src/geometry.ts); 40 is measured from its model and laid out as the
# 30: the window at ±13, the edge radius from ±17.5 and the band between.
LAYOUTS = {
    30: dict(flat=25.0, window=8.75, band=(8.75 + 12.5) / 2, centre=True),
    34: dict(flat=29.0, window=12.0, band=13.25, centre=False),
    40: dict(flat=35.0, window=13.0, band=(13.0 + 17.5) / 2, centre=True),
}
WORDMARK_MM = 0.95
TAGLINE_MM = 0.7
SERIAL_MM = 0.7
REGULATORY_MM = 0.62
SCRIPT_CLEAR_MM = 0.8

# The simulator's decal: rgb(150,152,156) at 55 %, roughness 0.7, metalness 0.3.
INK_SRGB = (150, 152, 156)
INK_AMOUNT = 0.55
INK_ROUGHNESS = 0.7
INK_METALLIC = 0.3
# How steep the marking's edges are in the normal map (rise per texel at full coverage).
EDGE_SLOPE = 0.7

TEXTURE_SIZE = 2048
# The line-up shows three dice side by side; smaller textures are plenty there.
LINEUP_TEXTURE_SIZE = 1024
NAME = "Etching"


def srgb_to_linear(c):
    c = np.asarray(c, dtype=np.float64)
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def linear_to_srgb(c):
    c = np.clip(np.asarray(c, dtype=np.float64), 0, 1)
    return np.where(c <= 0.0031308, c * 12.92, 1.055 * c ** (1 / 2.4) - 0.055)


# MARK: Drawing

def draw_etching(layout, n):
    """The etching's coverage (0-1) over the flat face: an n x n image, top
    row toward +Z (the lid's screen up), left toward -X. Laid out as the
    simulator's drawEtching does."""
    px = n / layout["flat"]
    band = layout["band"] * px
    window = layout["window"] * px

    def layer():
        return Image.new("L", (n, n), 0)

    def plain(img, text, mm):
        """A line on the top side, centred, its em box centred on the band.
        Returns the font, for measuring."""
        font = ImageFont.truetype(PLAIN_FONT, round(mm * px))
        ImageDraw.Draw(img).text((n / 2, n / 2 - band), text, fill=255, font=font, anchor="mm")
        return font

    def script(img, text, mm):
        """A script line on the top side, placed by its ink: centred in the band,
        or its lowest tail SCRIPT_CLEAR_MM off the window."""
        font = ImageFont.truetype(SCRIPT_FONT, round(mm * px))
        _, top, _, bottom = font.getbbox(text, anchor="ms")  # y down from the baseline
        if layout["centre"]:
            baseline = -band - (top + bottom) / 2
        else:
            baseline = -(window + SCRIPT_CLEAR_MM * px) - bottom
        ImageDraw.Draw(img).text((n / 2, n / 2 + baseline), text, fill=255, font=font, anchor="ms")

    out = np.zeros((n, n), dtype=np.float64)

    def add(img, turn):
        """Turns a layer drawn on the top side round to its own side, as the
        simulator's canvas rotations do, and adds it."""
        if turn is not None:
            img = img.transpose(turn)
        np.maximum(out, np.asarray(img, dtype=np.float64) / 255, out=out)

    top = layer()
    script(top, WORDMARK, WORDMARK_MM)
    add(top, None)

    right = layer()
    plain(right, f"{MODEL}  ·  S/N {SERIAL}", SERIAL_MM)
    add(right, Image.Transpose.ROTATE_270)  # a quarter turn clockwise

    bottom = layer()
    script(bottom, TAGLINE, TAGLINE_MM)
    add(bottom, Image.Transpose.ROTATE_180)

    # CE, a crossed-out wheelie bin, and where the rest lives. The bin sits
    # in the gap after CE (the simulator's -5.2 mm suits its own font's spaces).
    left = layer()
    line = "CE" + GAP + REGULATORY
    font = plain(left, line, REGULATORY_MM)
    start = n / 2 - font.getlength(line) / 2
    cx = start + font.getlength("CE") + font.getlength(GAP) / 2
    cy = n / 2 - band
    d = ImageDraw.Draw(left)
    s = 0.36 * px
    w = max(1, round(0.07 * px))
    d.rectangle([cx - s * 0.6, cy - s * 0.8, cx + s * 0.6, cy + s * 0.8], outline=255, width=w)
    d.line([cx - s, cy - s, cx + s, cy + s], fill=255, width=w)
    d.line([cx + s, cy - s, cx - s, cy + s], fill=255, width=w)
    add(left, Image.Transpose.ROTATE_90)  # a quarter turn anticlockwise

    # Only the band is marked: nothing over the window.
    mm = (np.arange(n) + 0.5 - n / 2) / px
    cheb = np.maximum(np.abs(mm)[None, :], np.abs(mm)[:, None])
    out[cheb < layout["window"]] = 0
    return out


def blur(a):
    """A small 3x3 blur, so the edges' slope spreads over a couple of texels."""
    k = (0.25, 0.5, 0.25)
    a = np.pad(a, 1, mode="edge")
    a = k[0] * a[:-2] + k[1] * a[1:-1] + k[2] * a[2:]
    return k[0] * a[:, :-2] + k[1] * a[:, 1:-1] + k[2] * a[:, 2:]


# MARK: Sampling the shell's material

def sample(img, u, v, wrap_s, wrap_t):
    """Bilinear samples of an image (H x W x C, floats) at USD texture
    coordinates (0, 0 at the bottom left)."""
    h, w = img.shape[:2]
    x = u * w - 0.5
    y = (1 - v) * h - 0.5
    x0, y0 = np.floor(x).astype(int), np.floor(y).astype(int)
    fx, fy = (x - x0)[..., None], (y - y0)[..., None]

    def wrap(i, n, mode):
        return i % n if mode == "repeat" else np.clip(i, 0, n - 1)

    xs = [wrap(x0, w, wrap_s), wrap(x0 + 1, w, wrap_s)]
    ys = [wrap(y0, h, wrap_t), wrap(y0 + 1, h, wrap_t)]
    top = img[ys[0], xs[0]] * (1 - fx) + img[ys[0], xs[1]] * fx
    low = img[ys[1], xs[0]] * (1 - fx) + img[ys[1], xs[1]] * fx
    return top * (1 - fy) + low * fy


class Surface:
    """The shell's UsdPreviewSurface: each input as a constant or a texture
    read through the shell's st."""

    def __init__(self, pbr, root_dir):
        self.pbr = pbr
        self.root_dir = root_dir

    def texture(self, name):
        i = self.pbr.GetInput(name)
        if not i or not i.HasConnectedSource():
            return None
        source, output, _ = i.GetConnectedSource()
        return UsdShade.Shader(source), output

    def value(self, name, default, st, channels):
        """The input at every st: an array (..., channels)."""
        tex = self.texture(name)
        if tex is None:
            i = self.pbr.GetInput(name)
            v = i.Get() if i and i.Get() is not None else default
            v = np.atleast_1d(np.array(v, dtype=np.float64))
            return np.broadcast_to(v, st.shape[:-1] + (len(v),)).copy()
        shader, output = tex

        def inp(k, default):
            x = shader.GetInput(k)
            return x.Get() if x and x.Get() is not None else default

        f = inp("file", None)
        if not f or not f.path:
            raise ValueError(f"{name}: texture without a file")
        img = Image.open(os.path.join(self.root_dir, f.path))
        mode = "RGBA" if img.mode in ("RGBA", "LA") else "RGB"
        a = np.asarray(img.convert(mode), dtype=np.float64) / 255
        if mode == "RGB":
            a = np.concatenate([a, np.ones(a.shape[:2] + (1,))], axis=-1)
        colour_space = inp("sourceColorSpace", "auto")
        if colour_space == "sRGB" or (colour_space == "auto" and name == "diffuseColor"):
            a[..., :3] = srgb_to_linear(a[..., :3])
        s = sample(a, st[..., 0], st[..., 1], inp("wrapS", "useMetadata"), inp("wrapT", "useMetadata"))
        s = s * np.array(inp("scale", Gf.Vec4f(1, 1, 1, 1))) + np.array(inp("bias", Gf.Vec4f(0, 0, 0, 0)))
        pick = {"rgb": [0, 1, 2], "r": [0], "g": [1], "b": [2], "a": [3]}[output]
        return s[..., pick]

    def normal_scale_bias(self):
        tex = self.texture("normal")
        if tex is None:
            return Gf.Vec4f(2, 2, 2, 1), Gf.Vec4f(-1, -1, -1, 0)
        sh = tex[0]
        sc, bi = sh.GetInput("scale"), sh.GetInput("bias")
        return (sc.Get() if sc and sc.Get() is not None else Gf.Vec4f(1, 1, 1, 1),
                bi.Get() if bi and bi.Get() is not None else Gf.Vec4f(0, 0, 0, 0))


# MARK: The model

def lid_faces(pts, counts, indices):
    """The faces lying flat in the shell's bottom plane: the lid round the window."""
    bottom = pts[:, 1].min()
    starts = np.concatenate([[0], np.cumsum(counts)[:-1]])
    flat = np.abs(pts[:, 1] - bottom) < 1e-7
    faces = [f for f, (s, c) in enumerate(zip(starts, counts)) if flat[indices[s:s + c]].all()]
    return faces, starts


def st_map(mesh, pts, faces, starts, counts, indices):
    """The shell's st on the lid as an affine map of (x, z): a 2 x 3 matrix,
    or None if it isn't one."""
    pv = UsdGeom.PrimvarsAPI(mesh.GetPrim()).GetPrimvar("st")
    if not pv or not pv.HasValue():
        return None
    st = np.array(pv.ComputeFlattened(), dtype=np.float64)
    face_varying = pv.GetInterpolation() == UsdGeom.Tokens.faceVarying
    corners = np.concatenate([np.arange(starts[f], starts[f] + counts[f]) for f in faces])
    p = pts[indices[corners]][:, [0, 2]]
    uv = st[corners] if face_varying else st[indices[corners]]
    a = np.c_[p, np.ones(len(p))]
    m, *_ = np.linalg.lstsq(a, uv, rcond=None)
    if np.abs(a @ m - uv).max() > 1e-4:
        return None
    return m.T  # uv = m @ (x, z, 1)


def lid_grid(layout, centre, n):
    """Each texel's (x, z) in metres, top row at +Z."""
    f = layout["flat"] / 1000
    t = (np.arange(n) + 0.5) / n - 0.5
    x = centre[0] + t[None, :] * f
    z = centre[1] - t[:, None] * f
    return np.broadcast_to(x, (n, n)), np.broadcast_to(z, (n, n))


def bake(surface, m, layout, centre, n):
    """The etched lid's colour, roughness/metallic and normal images."""
    x, z = lid_grid(layout, centre, n)
    st = np.stack([m[0, 0] * x + m[0, 1] * z + m[0, 2], m[1, 0] * x + m[1, 1] * z + m[1, 2]], axis=-1)
    cov = draw_etching(layout, n)
    t = (INK_AMOUNT * cov)[..., None]

    ink = srgb_to_linear(np.array(INK_SRGB) / 255)
    diffuse = surface.value("diffuseColor", (0.18, 0.18, 0.18), st, 3)
    colour = diffuse * (1 - t) + ink * t
    rough = surface.value("roughness", 0.5, st, 1) * (1 - t) + INK_ROUGHNESS * t
    metal = surface.value("metallic", 0.0, st, 1) * (1 - t) + INK_METALLIC * t

    # The shell's normal, from its UV frame on the lid into the new one (+u
    # along +X, +v along +Z). The columns of the inverse map are the
    # directions the old u and v run in (x, z).
    if surface.texture("normal") is not None:
        old = surface.value("normal", (0, 0, 1), st, 3)
        inv = np.linalg.inv(m[:, :2])
        du, dv = inv[:, 0] / np.linalg.norm(inv[:, 0]), inv[:, 1] / np.linalg.norm(inv[:, 1])
        nx = old[..., 0] * du[0] + old[..., 1] * dv[0]
        nz = old[..., 0] * du[1] + old[..., 1] * dv[1]
        base = np.stack([nx, nz, old[..., 2]], axis=-1)
    else:
        base = np.zeros((n, n, 3))
        base[..., 2] = 1
    # The marking as a slight recess. +v runs up the image, rows run down.
    h = -blur(cov)
    dh_du = (np.roll(h, -1, axis=1) - np.roll(h, 1, axis=1)) / 2
    dh_dv = -(np.roll(h, -1, axis=0) - np.roll(h, 1, axis=0)) / 2
    # Whiteout blend: the grain carries on through the marking.
    nrm = np.stack([base[..., 0] - dh_du * EDGE_SLOPE, base[..., 1] - dh_dv * EDGE_SLOPE, base[..., 2]], axis=-1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)

    def image(a, srgb=False):
        a = np.clip(linear_to_srgb(a) if srgb else a, 0, 1)
        return Image.fromarray((a * 255 + 0.5).astype(np.uint8), "RGB")

    rm = np.concatenate([rough, metal, np.zeros_like(rough)], axis=-1)
    return image(colour, srgb=True), image(rm), image(nrm * 0.5 + 0.5)


def etch_shell(stage, mesh, root_dir, n, report):
    name = mesh.GetPrim().GetPath()
    parent = mesh.GetPrim().GetParent()
    if parent.GetChild(NAME):
        report.append(f"{name}: already etched")
        return False
    if UsdGeom.Subset.GetAllGeomSubsets(mesh):
        report.append(f"{name}: has GeomSubsets, which this script doesn't remap; skipped")
        return False
    pts = np.array(mesh.GetPointsAttr().Get(), dtype=np.float64)
    counts = np.array(mesh.GetFaceVertexCountsAttr().Get())
    indices = np.array(mesh.GetFaceVertexIndicesAttr().Get())
    size_mm = round((pts[:, 0].max() - pts[:, 0].min()) * 1000)
    layout = LAYOUTS.get(size_mm)
    if layout is None:
        report.append(f"{name}: no etching layout for a {size_mm} mm die; skipped")
        return False

    faces, starts = lid_faces(pts, counts, indices)
    corners = np.concatenate([np.arange(starts[f], starts[f] + counts[f]) for f in faces])
    used = np.unique(indices[corners])
    # The die's centre: the line-up's dice sit side by side in one frame.
    centre = ((pts[:, 0].max() + pts[:, 0].min()) / 2, (pts[:, 2].max() + pts[:, 2].min()) / 2)
    lx, lz = (pts[used, 0] - centre[0]) * 1000, (pts[used, 2] - centre[1]) * 1000
    flat = np.maximum(np.abs(lx), np.abs(lz)).max()
    window = np.abs(lx[np.abs(lz) < 1.0]).min()  # across the middle; its corners are rounded
    if abs(flat - layout["flat"] / 2) > 0.05 or abs(window - layout["window"]) > 0.05:
        report.append(f"{name}: lid runs {window:.2f}-{flat:.2f} mm from the centre, "
                      f"expected {layout['window']}-{layout['flat'] / 2}; skipped")
        return False
    m = st_map(mesh, pts, faces, starts, counts, indices)
    if m is None:
        report.append(f"{name}: the shell's st on the lid isn't an affine map of x and z; skipped")
        return False

    material = UsdShade.MaterialBindingAPI(mesh.GetPrim()).ComputeBoundMaterial()[0]
    out = material.GetSurfaceOutput() if material else None
    pbr = UsdShade.Shader(out.GetConnectedSource()[0]) if out and out.HasConnectedSource() else None
    if not pbr or pbr.GetIdAttr().Get() != "UsdPreviewSurface":
        report.append(f"{name}: the shell has no UsdPreviewSurface to copy; skipped")
        return False
    surface = Surface(pbr, root_dir)

    # Textures, shared by every shell of this size and material in the file.
    stem = f"{NAME.lower()}_{size_mm}_{material.GetPath().name.lower()}"
    files = {kind: f"textures/{stem}_{kind}.png" for kind in ("colour", "rm", "normal")}
    if not all(os.path.exists(os.path.join(root_dir, f)) for f in files.values()):
        os.makedirs(os.path.join(root_dir, "textures"), exist_ok=True)
        for kind, img in zip(("colour", "rm", "normal"), bake(surface, m, layout, centre, n)):
            img.save(os.path.join(root_dir, files[kind]), optimize=True)
    mat_path = material.GetPath().GetParentPath().AppendChild(f"{material.GetPath().name}_{NAME}_{size_mm}")
    etched = UsdShade.Material(stage.GetPrimAtPath(mat_path))
    if not etched:
        etched = make_material(stage, mat_path, pbr, surface.normal_scale_bias(), files)

    # The lid's faces as their own mesh, on the shell's own points and normals.
    remap = {int(old): new for new, old in enumerate(used)}
    lid = UsdGeom.Mesh.Define(stage, parent.GetPath().AppendChild(NAME))
    lid_pts = pts[used]
    lid.CreatePointsAttr(Vt.Vec3fArray.FromNumpy(lid_pts.astype(np.float32)))
    lid.CreateFaceVertexCountsAttr(Vt.IntArray([int(counts[f]) for f in faces]))
    lid.CreateFaceVertexIndicesAttr(Vt.IntArray([remap[int(i)] for i in indices[corners]]))
    lid.CreateSubdivisionSchemeAttr(mesh.GetSubdivisionSchemeAttr().Get() or UsdGeom.Tokens.none)
    lid.CreateDoubleSidedAttr(mesh.GetDoubleSidedAttr().Get() or False)
    normals = mesh.GetNormalsAttr().Get()
    if normals is not None:
        normals = np.array(normals)
        interp = mesh.GetNormalsInterpolation()
        if interp == UsdGeom.Tokens.vertex:
            lid.CreateNormalsAttr(Vt.Vec3fArray.FromNumpy(normals[used].astype(np.float32)))
        elif interp == UsdGeom.Tokens.faceVarying:
            lid.CreateNormalsAttr(Vt.Vec3fArray.FromNumpy(normals[corners].astype(np.float32)))
        else:
            lid.CreateNormalsAttr(Vt.Vec3fArray.FromNumpy(normals.astype(np.float32)))
        lid.SetNormalsInterpolation(interp)
    f = layout["flat"] / 1000
    uv = np.c_[(lid_pts[:, 0] - centre[0]) / f + 0.5, (lid_pts[:, 2] - centre[1]) / f + 0.5]
    UsdGeom.PrimvarsAPI(lid.GetPrim()).CreatePrimvar(
        "st", Sdf.ValueTypeNames.TexCoord2fArray, UsdGeom.Tokens.vertex
    ).Set(Vt.Vec2fArray.FromNumpy(uv.astype(np.float32)))
    lo, hi = lid_pts.min(axis=0), lid_pts.max(axis=0)
    lid.CreateExtentAttr(Vt.Vec3fArray([Gf.Vec3f(*lo), Gf.Vec3f(*hi)]))
    xform = UsdGeom.Xformable(mesh.GetPrim())
    if xform.GetOrderedXformOps():
        lid.AddTransformOp().Set(xform.GetLocalTransformation())
    UsdShade.MaterialBindingAPI.Apply(lid.GetPrim()).Bind(etched)

    # And out of the shell. Its points stay as they are, so nothing else moves.
    remove_faces(mesh, set(faces), counts, starts, report)
    report.append(f"{name}: etched; {len(faces)} lid faces moved to {lid.GetPath()} ({size_mm} mm, {n} px)")
    return True


def remove_faces(mesh, drop, counts, starts, report):
    keep = [f for f in range(len(counts)) if f not in drop]
    indices = np.array(mesh.GetFaceVertexIndicesAttr().Get())
    corners = np.concatenate([np.arange(starts[f], starts[f] + counts[f]) for f in keep])
    mesh.GetFaceVertexCountsAttr().Set(Vt.IntArray([int(counts[f]) for f in keep]))
    mesh.GetFaceVertexIndicesAttr().Set(Vt.IntArray([int(i) for i in indices[corners]]))
    # Anything stored per face or per corner follows the faces it belongs to.
    if mesh.GetNormalsInterpolation() == UsdGeom.Tokens.faceVarying and mesh.GetNormalsAttr().Get() is not None:
        mesh.GetNormalsAttr().Set(Vt.Vec3fArray.FromNumpy(np.array(mesh.GetNormalsAttr().Get())[corners].astype(np.float32)))
    for pv in UsdGeom.PrimvarsAPI(mesh.GetPrim()).GetPrimvars():
        interp = pv.GetInterpolation()
        if interp not in (UsdGeom.Tokens.faceVarying, UsdGeom.Tokens.uniform):
            continue
        rows = corners if interp == UsdGeom.Tokens.faceVarying else np.array(keep)
        if pv.IsIndexed():
            pv.SetIndices(Vt.IntArray([int(i) for i in np.array(pv.GetIndices())[rows]]))
        else:
            values = pv.Get()
            pv.Set(type(values)([values[int(i)] for i in rows]))
        report.append(f"    trimmed {interp} primvar {pv.GetPrimvarName()}")


def make_material(stage, path, source_pbr, normal_scale_bias, files):
    """A copy of the shell's surface, with the baked textures connected."""
    mat = UsdShade.Material.Define(stage, path)
    pbr = UsdShade.Shader.Define(stage, path.AppendChild("PBR"))
    pbr.CreateIdAttr("UsdPreviewSurface")
    for i in source_pbr.GetInputs():
        if not i.HasConnectedSource() and i.Get() is not None:
            pbr.CreateInput(i.GetBaseName(), i.GetTypeName()).Set(i.Get())
    mat.CreateSurfaceOutput().ConnectToSource(pbr.ConnectableAPI(), "surface")

    reader = UsdShade.Shader.Define(stage, path.AppendChild("stReader"))
    reader.CreateIdAttr("UsdPrimvarReader_float2")
    reader.CreateInput("varname", Sdf.ValueTypeNames.String).Set("st")
    reader.CreateOutput("result", Sdf.ValueTypeNames.Float2)

    def texture(name, file, colour_space):
        t = UsdShade.Shader.Define(stage, path.AppendChild(name))
        t.CreateIdAttr("UsdUVTexture")
        t.CreateInput("file", Sdf.ValueTypeNames.Asset).Set(file)
        t.CreateInput("st", Sdf.ValueTypeNames.Float2).ConnectToSource(reader.ConnectableAPI(), "result")
        t.CreateInput("wrapS", Sdf.ValueTypeNames.Token).Set("clamp")
        t.CreateInput("wrapT", Sdf.ValueTypeNames.Token).Set("clamp")
        t.CreateInput("sourceColorSpace", Sdf.ValueTypeNames.Token).Set(colour_space)
        return t

    colour = texture("ColourTex", files["colour"], "sRGB")
    pbr.CreateInput("diffuseColor", Sdf.ValueTypeNames.Color3f).ConnectToSource(
        colour.CreateOutput("rgb", Sdf.ValueTypeNames.Float3))
    rm = texture("RoughMetalTex", files["rm"], "raw")
    pbr.CreateInput("roughness", Sdf.ValueTypeNames.Float).ConnectToSource(rm.CreateOutput("r", Sdf.ValueTypeNames.Float))
    pbr.CreateInput("metallic", Sdf.ValueTypeNames.Float).ConnectToSource(rm.CreateOutput("g", Sdf.ValueTypeNames.Float))
    normal = texture("NormalTex", files["normal"], "raw")
    # The image holds n * 0.5 + 0.5; decode as the shell's own normal map does.
    normal.CreateInput("scale", Sdf.ValueTypeNames.Float4).Set(Gf.Vec4f(2, 2, 2, 1))
    normal.CreateInput("bias", Sdf.ValueTypeNames.Float4).Set(Gf.Vec4f(-1, -1, -1, 0))
    pbr.CreateInput("normal", Sdf.ValueTypeNames.Normal3f).ConnectToSource(
        normal.CreateOutput("rgb", Sdf.ValueTypeNames.Float3))
    return mat


def package(root_dir, root_name, dst):
    """A usdz as the exports are made: the root layer first, then its files,
    stored (not compressed), each one's data aligned to 64 bytes."""
    files = [root_name] + sorted(
        os.path.relpath(os.path.join(d, f), root_dir).replace(os.sep, "/")
        for d, _, fs in os.walk(root_dir) for f in fs
        if os.path.relpath(os.path.join(d, f), root_dir).replace(os.sep, "/") != root_name
    )
    if os.path.exists(dst):
        os.remove(dst)
    with zipfile.ZipFile(dst, "w", zipfile.ZIP_STORED) as z:
        for name in files:
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            # The local header is 30 bytes plus the name, plus this padding.
            offset = z.fp.tell() + 30 + len(name.encode())
            pad = (-(offset + 4)) % 64
            info.extra = struct.pack("<HH", 0x1986, pad) + b"\0" * pad
            with open(os.path.join(root_dir, name), "rb") as f:
                z.writestr(info, f.read())


def referenced(root_dir, stage):
    """The files the stage points to, relative to the package."""
    out = set()
    for prim in stage.Traverse():
        for attr in prim.GetAttributes():
            if attr.GetTypeName() == Sdf.ValueTypeNames.Asset and attr.Get() and attr.Get().path:
                out.add(os.path.normpath(attr.Get().path).replace(os.sep, "/"))
    return out


def etch(src, dst, report):
    with tempfile.TemporaryDirectory() as tmp:
        tmp = os.path.realpath(tmp)
        with zipfile.ZipFile(src) as z:
            names = z.namelist()
            z.extractall(tmp)
        root = os.path.join(tmp, names[0])
        stage = Usd.Stage.Open(root)
        if stage is None:
            report.append(f"couldn't open {names[0]}")
            return False
        n = LINEUP_TEXTURE_SIZE if "lineup" in os.path.basename(src) else TEXTURE_SIZE
        shells = [UsdGeom.Mesh(p) for p in stage.Traverse() if p.IsA(UsdGeom.Mesh) and p.GetName() == "Shell"]
        changed = sum(etch_shell(stage, m, tmp, n, report) for m in shells)
        if not shells:
            report.append("no Shell mesh; copied unchanged")
        if not changed:
            shutil.copyfile(src, dst)
            return True
        stage.GetRootLayer().Save()
        missing = [f for f in referenced(tmp, stage) if not os.path.exists(os.path.join(tmp, f))]
        if missing:
            report.append(f"textures missing from the package: {', '.join(missing)}")
            return False
        package(tmp, names[0], dst)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("inputs", nargs="+", help="usdz files")
    parser.add_argument("--out", required=True, help="output folder")
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    failed = 0
    for src in args.inputs:
        dst = os.path.join(args.out, os.path.basename(src))
        if os.path.abspath(dst) == os.path.abspath(src):
            print(f"{src}: input and output are the same file; pick another --out")
            failed += 1
            continue
        report = []
        ok = etch(src, dst, report)
        print(f"[{'ok' if ok else 'FAIL'}] {os.path.basename(src)}")
        for line in report:
            print(f"    {line}")
        failed += not ok
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
