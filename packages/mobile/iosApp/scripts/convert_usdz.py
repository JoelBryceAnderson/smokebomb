#!/usr/bin/env python3
"""Rewrite hand-written USDZ models with a binary (.usdc) root and check them for ARKit.

The Sugarcube models are hand-written with a text (.usda) root layer.
RealityKit may load that, but ARKit's packaging rules expect a binary .usdc
root. For each input this script does the equivalent of
`usdcat in.usdz -o tmp.usdc` followed by `usdzip`:

1. It flattens the package's composed stage into one layer and saves that as .usdc.
2. It repackages that layer with its textures as an ARKit usdz.

Then it checks the result and reports problems. It never edits geometry.
The checks are:

- Package layout: a .usdc root, only ARKit file types, stored entries
  (not compressed), and data aligned to 64 bytes.
- Stage metadata: a defaultPrim, upAxis Y and metersPerUnit 1.
- Prim types and shader ids that ARKit / RealityKit accept.
- Every UsdValidation validator in this USD build.
- True size: the visual bounds match the size table to ±0.1 mm, with the
  origin at the bottom centre.
- `usdchecker --arkit` as well, when that tool is on PATH (for example
  Apple's USD tools, or a full USD build).

Setup (once):  python3 -m pip install -r requirements.txt
Usage:         python3 convert_usdz.py ~/Downloads/sugarcube_*.usdz
               python3 convert_usdz.py --check-only ../iosApp/Resources/Models/*.usdz

Output goes to ../iosApp/Resources/Models unless --out says otherwise. The exit
status is 1 if any model has errors. A model with errors is still written, so
you can inspect it, but it shouldn't be bundled.
"""

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zipfile

from pxr import Sdf, Usd, UsdGeom, UsdShade, UsdUtils

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_OUT = os.path.normpath(os.path.join(HERE, "..", "iosApp", "Resources", "Models"))

# Expected visual bounds in metres (x, y, z): the table in AR_VIEWER.md.
# Keep in step with ModelCatalog.swift.
EXPECTED_SIZE = {
    "sugarcube_30": (0.030, 0.030, 0.030),
    "sugarcube_30_rainbow": (0.030, 0.030, 0.030),
    "sugarcube_34": (0.034, 0.034, 0.034),
    "sugarcube_34_1bit": (0.034, 0.034, 0.034),
    "sugarcube_40": (0.040, 0.040, 0.040),
    "sugarcube_30_xray": (0.030, 0.030, 0.030),
    "sugarcube_34_xray": (0.034, 0.034, 0.034),
    "sugarcube_40_xray": (0.040, 0.040, 0.040),
    "sugarcube_30_xray_exploded": (0.0687, 0.0687, 0.0687),
    "sugarcube_lineup": (0.156, 0.040, 0.040),
    "sugarcube_contract_fixture": (0.030, 0.030, 0.030),
}
TOLERANCE_M = 0.0001  # ±0.1 mm

# What ARKit's usdz rules allow.
ALLOWED_EXTENSIONS = {".usdc", ".png", ".jpg", ".jpeg", ".m4a", ".mp3", ".wav"}
ALLOWED_PRIM_TYPES = {
    "", "Scope", "Xform", "Camera", "Shader", "Material", "NodeGraph", "Mesh", "Sphere", "Cube",
    "Cylinder", "Cone", "Capsule", "GeomSubset", "Points", "SkelRoot", "Skeleton",
    "SkelAnimation", "BlendShape", "SpatialAudio",
}
ALLOWED_SHADER_IDS = {"UsdPreviewSurface", "UsdUVTexture", "UsdTransform2d"}


class Report:
    def __init__(self, name):
        self.name = name
        self.errors = []
        self.warnings = []
        self.notes = []

    def error(self, msg):
        self.errors.append(msg)

    def warn(self, msg):
        self.warnings.append(msg)

    def note(self, msg):
        self.notes.append(msg)

    def print(self):
        status = "FAIL" if self.errors else "ok"
        print(f"\n[{status}] {self.name}")
        for n in self.notes:
            print(f"    {n}")
        for w in self.warnings:
            print(f"    warning: {w}")
        for e in self.errors:
            print(f"    ERROR: {e}")


def convert(src, dst, report):
    """Flatten src's stage to one binary layer and package it as an ARKit usdz at dst."""
    stem = os.path.splitext(os.path.basename(dst))[0]
    with tempfile.TemporaryDirectory() as tmp:
        # macOS's temp folder is behind a symlink (/var -> /private/var); resolve it so
        # texture paths anchor consistently while packaging.
        tmp = os.path.realpath(tmp)
        # Unpack the package so textures keep their relative paths next to the new root.
        with zipfile.ZipFile(src) as z:
            names = z.namelist()
            if not names:
                report.error("the package is empty")
                return False
            z.extractall(tmp)
        root = os.path.join(tmp, names[0])
        report.note(f"source root layer: {names[0]}")
        stage = Usd.Stage.Open(root)
        if stage is None:
            report.error(f"couldn't open {names[0]}")
            return False
        flat = stage.Flatten()
        fixed = apply_material_binding_api(flat)
        if fixed:
            report.note(f"applied MaterialBindingAPI to {fixed} prim(s) that bind materials (metadata only; geometry untouched)")
        usdc = os.path.join(tmp, f"{stem}.usdc")
        # The .usdc extension selects the binary crate format.
        if not flat.Export(usdc):
            report.error("couldn't write the binary layer")
            return False
        if os.path.exists(dst):
            os.remove(dst)
        if not UsdUtils.CreateNewARKitUsdzPackage(usdc, dst):
            report.error("UsdUtils.CreateNewARKitUsdzPackage failed")
            return False
    return True


def apply_material_binding_api(layer):
    """Declare MaterialBindingAPI on prims that have material:binding relationships but don't
    declare it. Current USD and ARKit require the schema; adding it changes no geometry or look.
    Returns how many prims were changed."""
    stage = Usd.Stage.Open(layer)
    fixed = 0
    for prim in stage.Traverse():
        binds = any(r.GetName().startswith("material:binding") for r in prim.GetRelationships())
        if binds and not prim.HasAPI(UsdShade.MaterialBindingAPI):
            UsdShade.MaterialBindingAPI.Apply(prim)
            fixed += 1
    return fixed


def check_package(path, report):
    with zipfile.ZipFile(path) as z:
        infos = z.infolist()
        if not infos:
            report.error("empty package")
            return
        root = infos[0].filename
        if not root.endswith(".usdc"):
            report.error(f"root layer is {root}, not a binary .usdc")
        with open(path, "rb") as f:
            for info in infos:
                ext = os.path.splitext(info.filename)[1].lower()
                if ext not in ALLOWED_EXTENSIONS:
                    report.error(f"{info.filename}: file type ARKit won't read in a usdz")
                if info.compress_type != zipfile.ZIP_STORED:
                    report.error(f"{info.filename}: compressed (usdz entries must be stored)")
                # Data starts after the 30-byte local header, name and extra field.
                f.seek(info.header_offset + 26)
                name_len, extra_len = struct.unpack("<HH", f.read(4))
                data_offset = info.header_offset + 30 + name_len + extra_len
                if data_offset % 64:
                    report.error(f"{info.filename}: data not 64-byte aligned")
        report.note(f"package: {len(infos)} file(s), root {root}")


def check_stage(path, stem, report):
    stage = Usd.Stage.Open(path)
    if stage is None:
        report.error("couldn't open the converted package")
        return

    default = stage.GetDefaultPrim()
    if not default:
        report.error("no defaultPrim")
    up = UsdGeom.GetStageUpAxis(stage)
    if up != UsdGeom.Tokens.y:
        report.error(f"upAxis is {up}, expected Y")
    mpu = UsdGeom.GetStageMetersPerUnit(stage)
    if not stage.HasAuthoredMetadata("metersPerUnit"):
        report.warn("metersPerUnit isn't authored (USD then assumes centimetres)")
    elif abs(mpu - 1.0) > 1e-9:
        report.error(f"metersPerUnit is {mpu}, expected 1")

    for prim in stage.Traverse():
        t = prim.GetTypeName()
        if t not in ALLOWED_PRIM_TYPES:
            report.error(f"{prim.GetPath()}: prim type {t} isn't supported by ARKit")
        if t == "Shader":
            sid = UsdShade.Shader(prim).GetIdAttr().Get()
            if sid not in ALLOWED_SHADER_IDS and not str(sid).startswith("UsdPrimvarReader_"):
                report.error(f"{prim.GetPath()}: shader id {sid} isn't supported by ARKit")

    check_textures(stage, report)
    run_validators(stage, report)
    check_bounds(stage, stem, mpu, report)
    report_prims(stage, report)


def check_textures(stage, report):
    """Every texture the model uses must resolve to a file inside the package."""
    count = 0
    for prim in stage.Traverse():
        for attr in prim.GetAttributes():
            if attr.GetTypeName() != Sdf.ValueTypeNames.Asset:
                continue
            value = attr.Get()
            if not value or not value.path:
                continue
            count += 1
            if not value.resolvedPath:
                report.error(f"{attr.GetPath()}: texture {value.path} isn't in the package")
    report.note(f"textures: {count} referenced, all checked")


def run_validators(stage, report):
    try:
        from pxr import UsdValidation
    except ImportError:
        report.warn("this USD build has no UsdValidation; skipped its validators")
        return
    registry = UsdValidation.ValidationRegistry()
    validators = registry.GetOrLoadAllValidators()
    errors = UsdValidation.ValidationContext(validators).Validate(stage)
    for e in errors:
        if e.HasNoError():
            continue
        kind = str(e.GetType())
        msg = f"{e.GetIdentifier()}: {e.GetMessage()}"
        if "Error" in kind:
            report.error(msg)
        else:
            report.warn(msg)
    report.note(f"UsdValidation: ran {len(validators)} validators")


def check_bounds(stage, stem, mpu, report):
    cache = UsdGeom.BBoxCache(Usd.TimeCode.Default(), [UsdGeom.Tokens.default_, UsdGeom.Tokens.render])
    box = cache.ComputeWorldBound(stage.GetPseudoRoot()).ComputeAlignedRange()
    if box.IsEmpty():
        report.error("no visible geometry")
        return
    lo, hi = box.GetMin() * mpu, box.GetMax() * mpu
    size = hi - lo
    report.note(
        "bounds: {:.2f} x {:.2f} x {:.2f} mm, min y {:+.2f} mm, centre x/z {:+.2f}/{:+.2f} mm".format(
            size[0] * 1000, size[1] * 1000, size[2] * 1000, lo[1] * 1000,
            (lo[0] + hi[0]) * 500, (lo[2] + hi[2]) * 500,
        )
    )
    expected = EXPECTED_SIZE.get(stem)
    if expected is None:
        report.warn(f"{stem} isn't in the size table; add it here and to ModelCatalog.swift")
        return
    for axis, name in enumerate("xyz"):
        if abs(size[axis] - expected[axis]) > TOLERANCE_M:
            report.error(
                f"{name} size {size[axis] * 1000:.2f} mm, expected {expected[axis] * 1000:.1f} ± 0.1 mm"
            )
    if abs(lo[1]) > TOLERANCE_M:
        report.error(f"bottom is at y = {lo[1] * 1000:+.2f} mm; the origin should be at the bottom")
    for axis, name in ((0, "x"), (2, "z")):
        if abs(lo[axis] + hi[axis]) / 2 > TOLERANCE_M:
            report.error(f"not centred on {name}: centre at {(lo[axis] + hi[axis]) * 500:+.2f} mm")


def report_prims(stage, report):
    """List the contract prims the viewer will find, so a re-export is easy to verify."""
    default = stage.GetDefaultPrim()
    if not default:
        return
    names = {p.GetName() for p in Usd.PrimRange(default)}
    faces = ["px", "nx", "py", "ny", "pz", "nz"]
    per_part = [f"Window_{f}" for f in faces] + [f"Module_{f}" for f in faces]
    found = [n for n in per_part if n in names]
    if found and len(found) < len(per_part):
        missing = sorted(set(per_part) - names)
        report.warn(f"partly per-part: missing {', '.join(missing)}")
    contract = ["Shell", "Lid", "Board", "Cell", "BalancePlate", "Wiring"]
    contract += [f"Screw_{i}" for i in range(4)] + [f"Pillar_{i}" for i in range(4)]
    present = [n for n in contract if n in names]
    report.note(
        f"per-part prims: {len(found)}/12 windows+modules, {len(present)}/{len(contract)} other contract parts"
    )


def run_usdchecker(path, report):
    tool = shutil.which("usdchecker")
    if not tool:
        report.note("usdchecker not on PATH; ran the built-in checks only")
        return
    proc = subprocess.run([tool, "--arkit", path], capture_output=True, text=True)
    out = (proc.stdout + proc.stderr).strip()
    if proc.returncode != 0:
        report.error("usdchecker --arkit failed:\n        " + out.replace("\n", "\n        "))
    else:
        report.note("usdchecker --arkit: passed")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("inputs", nargs="+", help="usdz files")
    parser.add_argument("--out", default=DEFAULT_OUT, help=f"output folder (default {DEFAULT_OUT})")
    parser.add_argument("--check-only", action="store_true", help="check the inputs as they are; write nothing")
    args = parser.parse_args()

    if not args.check_only:
        os.makedirs(args.out, exist_ok=True)
    failed = 0
    for src in args.inputs:
        stem = os.path.splitext(os.path.basename(src))[0]
        report = Report(stem)
        if args.check_only:
            path = src
        else:
            path = os.path.join(args.out, f"{stem}.usdz")
            if os.path.abspath(path) == os.path.abspath(src):
                report.error("input and output are the same file; use --out or --check-only")
                report.print()
                failed += 1
                continue
            if not convert(src, path, report):
                report.print()
                failed += 1
                continue
        check_package(path, report)
        check_stage(path, stem, report)
        run_usdchecker(path, report)
        report.print()
        failed += bool(report.errors)

    print(f"\n{len(args.inputs) - failed}/{len(args.inputs)} model(s) passed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
