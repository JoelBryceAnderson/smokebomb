#!/usr/bin/env python3
"""Draws the Sugarcube app icon and writes every iOS and Android size.

The icon is the boot screen's finale in one frame: the wordmark's script
(Pacifico, the face the die writes "Sugarcube" in) as a monogram S, with the
gold glint on its tail and a few sugar crystals drifting off. Colours are the
64x64 panel's boot ink (firmware/core/src/palette64.rs) on the app's
background (composeApp theme).

Needs Pillow and fontTools:  pip install pillow fonttools
Run from anywhere:           python3 packages/mobile/scripts/app_icon.py
"""

import io
import math
import random
from pathlib import Path

from fontTools.ttLib import TTFont
from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

MOBILE = Path(__file__).resolve().parent.parent
REPO = MOBILE.parent.parent
FONT = REPO / "packages/firmware/assets/fonts/pacifico-latin-400-normal.woff"
IOS_SET = MOBILE / "iosApp/iosApp/Assets.xcassets/AppIcon.appiconset"
ANDROID_RES = MOBILE / "composeApp/src/androidMain/res"

# palette64: WHITE (word, pips), SUGAR (crystals), GOLD (glint).
WHITE = (0xF4, 0xF5, 0xF7)
SUGAR = (0xD8, 0xD4, 0xFF)
GOLD = (0xF5, 0xC4, 0x51)
# Theme.kt: surfaceVariant at the centre falling to background at the edge.
BG_CENTRE = (0x24, 0x24, 0x2D)
BG_EDGE = (0x0D, 0x0D, 0x10)

# Everything is drawn on a 1024 canvas at 4x and downsampled.
SIZE = 1024
SS = 4
# The glyph's height as a fraction of the canvas. Android's adaptive
# foreground keeps it inside the 66% safe circle (see android_foreground).
GLYPH_H = 0.60


def pacifico(px):
    font = TTFont(io.BytesIO(FONT.read_bytes()))
    font.flavor = None
    buf = io.BytesIO()
    font.save(buf)
    buf.seek(0)
    return ImageFont.truetype(buf, px)


def background(n):
    """Radial falloff from centre to edge, slightly above centre like a lit face."""
    img = Image.new("RGB", (n, n))
    px = img.load()
    cx, cy, r = n / 2, n * 0.42, n * 0.78
    for y in range(n):
        for x in range(n):
            t = min(1.0, math.hypot(x - cx, y - cy) / r)
            t = t * t * (3 - 2 * t)
            px[x, y] = tuple(round(a + (b - a) * t) for a, b in zip(BG_CENTRE, BG_EDGE))
    return img


def glyph_mask(n, height):
    """The S as an alpha mask, its ink box centred on the canvas, `height` px tall."""
    probe = pacifico(1000)
    l, t, r, b = probe.getbbox("S")
    font = pacifico(round(1000 * height / (b - t)))
    l, t, r, b = font.getbbox("S")
    mask = Image.new("L", (n, n), 0)
    # Optical centre: the S leans right, so nudge it back a touch.
    ox = (n - (r - l)) / 2 - l - n * 0.01
    oy = (n - (b - t)) / 2 - t
    ImageDraw.Draw(mask).text((ox, oy), "S", font=font, fill=255)
    return mask


def sparkle(draw, cx, cy, r, colour, alpha=255):
    """A four-point star: two thin diamonds crossed, like the glint's twinkle."""
    w = r * 0.22
    fill = colour + (alpha,)
    draw.polygon([(cx, cy - r), (cx + w, cy), (cx, cy + r), (cx - w, cy)], fill=fill)
    draw.polygon([(cx - r, cy), (cx, cy - w), (cx + r, cy), (cx, cy + w)], fill=fill)


def crystal(draw, cx, cy, s, angle, colour, alpha):
    """A small rotated square: a grain of the dissolved cube."""
    pts = []
    for k in range(4):
        a = angle + k * math.pi / 2
        pts.append((cx + s * math.cos(a), cy + s * math.sin(a)))
    draw.polygon(pts, fill=colour + (alpha,))


def ornaments(n, mask, glyph_colour, sugar, gold):
    """Glint and crystals on a transparent layer, placed relative to the S."""
    layer = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    l, t, r, b = mask.getbbox()
    w, h = r - l, b - t
    # Glint where the pen lifts: the end of the S's lower terminal.
    if gold:
        gx, gy = l + w * 0.02, t + h * 0.6
        sparkle(d, gx, gy, n * 0.07, gold)
        sparkle(d, gx, gy, n * 0.028, glyph_colour)
    # Crystals drift up and right off the top of the S.
    if sugar:
        rng = random.Random(7)
        for i, (fx, fy, fs) in enumerate(
            [(1.02, 0.02, 0.022), (1.14, -0.06, 0.016), (1.10, 0.14, 0.013),
             (1.24, 0.06, 0.010), (0.94, -0.10, 0.012), (1.22, -0.14, 0.008)]
        ):
            alpha = round(255 * (1.0 - 0.12 * i))
            crystal(d, l + w * fx, t + h * fy, n * fs, rng.uniform(0, math.pi / 2), sugar, alpha)
    return layer


def glow(mask, colour, radius, strength):
    halo = mask.filter(ImageFilter.GaussianBlur(radius))
    halo = halo.point(lambda v: min(255, round(v * strength)))
    layer = Image.new("RGBA", mask.size, colour + (0,))
    layer.putalpha(halo)
    return layer


def compose(glyph_h, bg, glyph_colour=WHITE, sugar=SUGAR, gold=GOLD, halo=True):
    """The icon at 1024 px. `bg` is an RGB image, or None for transparent."""
    n = SIZE * SS
    mask = glyph_mask(n, n * glyph_h)
    out = Image.new("RGBA", (n, n), (0, 0, 0, 0)) if bg is None else bg.resize((n, n)).convert("RGBA")
    if halo:
        out.alpha_composite(glow(mask, glyph_colour, n * 0.02, 0.55))
    ink = Image.new("RGBA", (n, n), glyph_colour + (0,))
    ink.putalpha(mask)
    out.alpha_composite(ink)
    orn = ornaments(n, mask, glyph_colour, sugar, gold)
    if halo:
        out.alpha_composite(glow(orn.getchannel("A"), gold or glyph_colour, n * 0.012, 0.6))
    out.alpha_composite(orn)
    return out.resize((SIZE, SIZE), Image.LANCZOS)


def save(img, path, size=None, opaque=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    if size:
        img = img.resize((size, size), Image.LANCZOS)
    if opaque:
        img = img.convert("RGB")
    img.save(path, optimize=True)
    print(path.relative_to(REPO))


def ios():
    bg = background(SIZE)
    save(compose(GLYPH_H, bg), IOS_SET / "AppIcon.png", opaque=True)
    # iOS 18 dark: the system draws its own dark backdrop behind a transparent icon.
    save(compose(GLYPH_H, None), IOS_SET / "AppIcon-Dark.png")
    # iOS 18 tinted: greyscale, the system applies the tint.
    tinted = compose(GLYPH_H, Image.new("RGB", (SIZE, SIZE), (0, 0, 0)),
                     glyph_colour=(255, 255, 255), sugar=(200, 200, 200), gold=(235, 235, 235), halo=False)
    save(tinted.convert("L").convert("RGB"), IOS_SET / "AppIcon-Tinted.png", opaque=True)
    (IOS_SET / "Contents.json").write_text(IOS_CONTENTS)
    (IOS_SET.parent / "Contents.json").write_text('{\n  "info" : {\n    "author" : "xcode",\n    "version" : 1\n  }\n}\n')


IOS_CONTENTS = """{
  "images" : [
    {
      "filename" : "AppIcon.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    },
    {
      "appearances" : [
        {
          "appearance" : "luminosity",
          "value" : "dark"
        }
      ],
      "filename" : "AppIcon-Dark.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    },
    {
      "appearances" : [
        {
          "appearance" : "luminosity",
          "value" : "tinted"
        }
      ],
      "filename" : "AppIcon-Tinted.png",
      "idiom" : "universal",
      "platform" : "ios",
      "size" : "1024x1024"
    }
  ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  }
}
"""

# Legacy launcher sizes (48 dp) and adaptive layer sizes (108 dp) per density.
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def android():
    # Adaptive layers are 108 dp with the visible mask inside the middle 72 dp
    # and a 66 dp safe circle, so the S is scaled to sit within that.
    fg_h = GLYPH_H * 66 / 108 * 0.95
    fg = compose(fg_h, None)
    mono = compose(fg_h, None, glyph_colour=(255, 255, 255), sugar=(255, 255, 255), gold=(255, 255, 255), halo=False)
    bg = background(SIZE)
    legacy = compose(GLYPH_H, bg)
    for name, k in DENSITIES.items():
        d = ANDROID_RES / f"mipmap-{name}"
        save(fg, d / "ic_launcher_foreground.png", round(108 * k))
        save(mono, d / "ic_launcher_monochrome.png", round(108 * k))
        save(bg, d / "ic_launcher_background.png", round(108 * k), opaque=True)
        save(rounded(legacy, 0.18), d / "ic_launcher.png", round(48 * k))
        save(rounded(legacy, 0.5), d / "ic_launcher_round.png", round(48 * k))
    anydpi = ANDROID_RES / "mipmap-anydpi-v26"
    anydpi.mkdir(parents=True, exist_ok=True)
    for name in ("ic_launcher", "ic_launcher_round"):
        (anydpi / f"{name}.xml").write_text(ADAPTIVE)
        print((anydpi / f"{name}.xml").relative_to(REPO))


def rounded(img, radius):
    """Pre-Oreo launchers show the PNG as is, so it carries its own shape."""
    n = img.width * SS
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, n - 1, n - 1), radius=n * radius, fill=255)
    out = img.copy()
    out.putalpha(ImageChops.multiply(img.getchannel("A"), mask.resize(img.size, Image.LANCZOS)))
    return out


ADAPTIVE = """<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by packages/mobile/scripts/app_icon.py -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background" />
    <foreground android:drawable="@mipmap/ic_launcher_foreground" />
    <monochrome android:drawable="@mipmap/ic_launcher_monochrome" />
</adaptive-icon>
"""

if __name__ == "__main__":
    ios()
    android()
