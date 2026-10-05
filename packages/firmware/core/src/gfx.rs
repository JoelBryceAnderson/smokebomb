//! Drawing primitives for the panels, in the mockup's canvas units.
//!
//! The mockup draws each face on a 256×256 canvas whose lit area is ±83 units
//! around the centre, then downsamples it to 96×96 (SIM_SPEC B1). Screens here
//! are written in those canvas units, relative to the face centre (x right,
//! y down), so layouts port directly from the mockup. A [`Transform`] maps
//! them to panel pixels with the same order the mockup uses:
//! `translate(centre + offset); rotate(angle); scale(s)`.
//!
//! The same code draws on every display target ([`smokebomb_hal::Target`]):
//! a [`Transform`] carries its target's scale, centre and size, the
//! [`Layer`] its scratch planes, and [`Style`] a colour that each target's
//! pixel type reduces (grey: brightest channel; RGB565: top bits). On the
//! 96×96 target this is exactly the original grey pipeline.
//!
//! Every draw call rasterizes anti-aliased coverage into a scratch layer, then
//! composites it with source-over blending. A non-zero `glow` reproduces the
//! canvas `shadowBlur` halo: the layer is blurred (three box passes ≈ a
//! Gaussian with σ = blur / 2) and composited underneath first.

use libm::{cosf, fabsf, floorf, roundf, sinf, sqrtf};
use smokebomb_hal::{Color, Grey96, Target};

use crate::display::Framebuffer;
use crate::orientation::Quarter;

/// Panel pixels per canvas unit on the 96×96 target: the lit ±83 units span
/// 96 pixels.
pub const K: f32 = 96.0 / 166.0;
/// Panel centre on the 96×96 target, in pixel coordinates (pixel `i` covers
/// `[i, i + 1)`).
pub const CENTER: f32 = 48.0;

/// Panel pixels per canvas unit on `T`: the lit ±83 canvas units span the
/// panel, whatever its size.
pub const fn k_of<T: Target>() -> f32 {
    T::WIDTH as f32 / 166.0
}

/// Panel centre on `T`.
pub const fn center_of<T: Target>() -> f32 {
    T::WIDTH as f32 / 2.0
}

/// Canvas units (face-centred, y down) → panel pixels.
#[derive(Clone, Copy, Debug)]
pub struct Transform {
    cos: f32,
    sin: f32,
    scale: f32,
    /// Offset of the face centre, panel px (applied before rotation, as the
    /// mockup's `translate(S/2 + ox, …)`).
    ox: f32,
    oy: f32,
    /// The target: panel px per canvas unit, its centre, and its size.
    k: f32,
    center: f32,
    width: usize,
    height: usize,
}

impl Default for Transform {
    fn default() -> Self {
        Self::rotated(0.0)
    }
}

impl Transform {
    /// Rotated by `angle` on the 96×96 target.
    pub fn rotated(angle: f32) -> Self {
        Self::rotated_on::<Grey96>(angle)
    }

    /// Rotated by `angle` on target `T`.
    pub fn rotated_on<T: Target>(angle: f32) -> Self {
        Self::new::<T>(cosf(angle), sinf(angle))
    }

    /// A quarter turn on the 96×96 target.
    pub fn quarter(q: Quarter) -> Self {
        Self::quarter_on::<Grey96>(q)
    }

    /// A quarter turn on target `T`.
    pub fn quarter_on<T: Target>(q: Quarter) -> Self {
        let (cos, sin) = match q {
            Quarter::R0 => (1.0, 0.0),
            Quarter::R90 => (0.0, 1.0),
            Quarter::R180 => (-1.0, 0.0),
            Quarter::R270 => (0.0, -1.0),
        };
        Self::new::<T>(cos, sin)
    }

    fn new<T: Target>(cos: f32, sin: f32) -> Self {
        Self {
            cos,
            sin,
            scale: 1.0,
            ox: 0.0,
            oy: 0.0,
            k: k_of::<T>(),
            center: center_of::<T>(),
            width: T::WIDTH,
            height: T::HEIGHT,
        }
    }

    /// The same orientation and offset, with one unit a whole panel pixel
    /// (times any [`Self::scaled`] so far) instead of a canvas unit: for
    /// layouts drawn pixel by pixel (the 64×64 screens). Origin stays at
    /// the panel centre; offsets made before stay in canvas units.
    pub fn in_pixels(mut self) -> Self {
        self.scale /= self.k;
        self
    }

    /// The panel's size in pixels.
    pub fn panel(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Scale about the (offset) centre, after rotation.
    pub fn scaled(mut self, s: f32) -> Self {
        self.scale *= s;
        self
    }

    /// Move the centre by canvas units in unrotated face space.
    pub fn offset(mut self, dx: f32, dy: f32) -> Self {
        self.ox += dx * self.k;
        self.oy += dy * self.k;
        self
    }

    /// Panel pixels per canvas unit.
    pub fn px_per_unit(&self) -> f32 {
        self.scale * self.k
    }

    pub fn forward(&self, x: f32, y: f32) -> (f32, f32) {
        let s = self.px_per_unit();
        (
            self.center + self.ox + (x * self.cos - y * self.sin) * s,
            self.center + self.oy + (x * self.sin + y * self.cos) * s,
        )
    }

    pub fn inverse(&self, px: f32, py: f32) -> (f32, f32) {
        let s = self.px_per_unit();
        let (dx, dy) = ((px - self.center - self.ox) / s, (py - self.center - self.oy) / s);
        (dx * self.cos + dy * self.sin, -dx * self.sin + dy * self.cos)
    }

    /// Pixel bounds covering a canvas-space rectangle (plus `pad` px).
    pub fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32, pad: f32) -> Rect {
        let corners = [
            self.forward(x0, y0),
            self.forward(x1, y0),
            self.forward(x0, y1),
            self.forward(x1, y1),
        ];
        let (mut lx, mut ly, mut hx, mut hy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in corners {
            lx = lx.min(x);
            ly = ly.min(y);
            hx = hx.max(x);
            hy = hy.max(y);
        }
        Rect::clip_to(lx - pad, ly - pad, hx + pad, hy + pad, self.width, self.height)
    }
}

/// Pixel rectangle `[x0, x1) × [y0, y1)`, clipped to the panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Rect {
    /// Clipped to the 96×96 panel.
    pub fn clip(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Self::clip_to(x0, y0, x1, y1, Grey96::WIDTH, Grey96::HEIGHT)
    }

    /// Clipped to a `width` × `height` panel.
    pub fn clip_to(x0: f32, y0: f32, x1: f32, y1: f32, width: usize, height: usize) -> Rect {
        let c = |v: f32, max: usize| -> usize { (floorf(v).max(0.0) as usize).min(max) };
        Rect {
            x0: c(x0, width),
            y0: c(y0, height),
            x1: c(x1 + 1.0, width),
            y1: c(y1 + 1.0, height),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    pub fn union(self, o: Rect) -> Rect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        Rect {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    /// Grown by `r` on every side, within a `width` × `height` panel.
    pub fn grow_in(self, r: usize, width: usize, height: usize) -> Rect {
        Rect {
            x0: self.x0.saturating_sub(r),
            y0: self.y0.saturating_sub(r),
            x1: (self.x1 + r).min(width),
            y1: (self.y1 + r).min(height),
        }
    }
}

/// How a shape is painted.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    /// The colour. A grey panel shows its brightest channel, which is how
    /// the mockup's quantiser reads its colours.
    pub color: Color,
    /// Canvas `globalAlpha`, 0–1.
    pub alpha: f32,
    /// Canvas `shadowBlur` in canvas units; 0 for none.
    pub glow: f32,
}

impl Style {
    /// Grey `value` (the brightest channel of the mockup's colour).
    pub const fn new(value: u8, alpha: f32, glow: f32) -> Self {
        Self {
            color: Color::grey(value),
            alpha,
            glow,
        }
    }

    pub const fn color(color: Color, alpha: f32, glow: f32) -> Self {
        Self { color, alpha, glow }
    }

    /// The same, at `alpha` times its opacity.
    pub fn faded(self, alpha: f32) -> Self {
        Self {
            alpha: self.alpha * alpha,
            ..self
        }
    }
}

/// Scratch buffers shared by all faces: coverage, blurred copy, temp.
pub struct Layer<T: Target = Grey96> {
    cov: T::Plane,
    blur: T::Plane,
    tmp: T::Plane,
    dirty: Rect,
}

impl<T: Target> Default for Layer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Target> Layer<T> {
    pub const fn new() -> Self {
        Self {
            cov: T::BLANK_PLANE,
            blur: T::BLANK_PLANE,
            tmp: T::BLANK_PLANE,
            dirty: Rect {
                x0: 0,
                y0: 0,
                x1: 0,
                y1: 0,
            },
        }
    }
}

pub struct Painter<'a, T: Target = Grey96> {
    fb: &'a mut Framebuffer<T>,
    layer: &'a mut Layer<T>,
    pub xf: Transform,
    /// Every text (and text-sized icon) drawn, for the size audit.
    pub marks: Marks,
    /// What the next marks are ([`MarkKind::Text`] unless a screen says).
    pub mark_kind: MarkKind,
}

/// What a mark is, for the size audit (brief 3, 1.2–1.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    /// Text: its cap height and length count.
    Text,
    /// A picture standing in for text (a T1 glyph, a T2 icon).
    Icon,
    /// A tap's hint, which may be held-sized on a table screen (2.1).
    Hint,
}

/// One text drawn on a face: its cap height in panel px, how many
/// characters, and its alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub cap_px: f32,
    /// How wide the text is, panel px (0 for an icon).
    pub width_px: f32,
    pub chars: u8,
    pub alpha: f32,
    pub kind: MarkKind,
}

/// A face's marks. Past the capacity, marks are dropped (the audit fails
/// any face that fills it: no screen should come near).
pub type Marks = heapless::Vec<Mark, 12>;

impl<'a, T: Target> Painter<'a, T> {
    /// `xf` should be made for `T` ([`Transform::quarter_on`]).
    pub fn new(fb: &'a mut Framebuffer<T>, layer: &'a mut Layer<T>, xf: Transform) -> Self {
        debug_assert_eq!(
            xf.panel(),
            (T::WIDTH, T::HEIGHT),
            "transform made for another target"
        );
        Self {
            fb,
            layer,
            xf,
            marks: Marks::new(),
            mark_kind: MarkKind::Text,
        }
    }

    /// Note text `cap` drawing units tall (cap height × scale), `width`
    /// units wide and `chars` long, for the size audit.
    pub fn note_text(&mut self, cap: f32, width: f32, chars: usize, alpha: f32) {
        let kind = self.mark_kind;
        self.note(kind, cap, width, chars, alpha);
    }

    /// Note a picture `height` drawing units tall standing in for text.
    pub fn note_icon(&mut self, height: f32, alpha: f32) {
        self.note(MarkKind::Icon, height, 0.0, 1, alpha);
    }

    fn note(&mut self, kind: MarkKind, cap: f32, width: f32, chars: usize, alpha: f32) {
        if alpha <= 0.0 || chars == 0 {
            return;
        }
        let _ = self.marks.push(Mark {
            cap_px: cap * self.xf.px_per_unit(),
            width_px: width * self.xf.px_per_unit(),
            chars: chars.min(255) as u8,
            alpha,
            kind,
        });
    }

    /// The framebuffer, for code that still draws directly (placeholders).
    pub fn framebuffer(&mut self) -> &mut Framebuffer<T> {
        self.fb
    }

    /// Start a draw call: clear what the previous one left in the layer.
    pub fn begin(&mut self) {
        let d = self.layer.dirty;
        let w = T::WIDTH;
        let cov = self.layer.cov.as_mut();
        for y in d.y0..d.y1 {
            cov[y * w + d.x0..y * w + d.x1].fill(0);
        }
        self.layer.dirty = Rect::default();
    }

    /// Add coverage (0–1) at a pixel; overlapping shapes in one call union.
    pub fn cover(&mut self, x: usize, y: usize, c: f32) {
        if c <= 0.0 {
            return;
        }
        let v = (c.min(1.0) * 255.0 + 0.5) as u8;
        let p = &mut self.layer.cov.as_mut()[y * T::WIDTH + x];
        if v > *p {
            *p = v;
        }
    }

    pub fn mark(&mut self, r: Rect) {
        self.layer.dirty = self.layer.dirty.union(r);
    }

    /// Composite the layer: glow first, then the shape (source-over).
    pub fn finish(&mut self, style: Style) {
        let d = self.layer.dirty;
        if d.is_empty() || style.alpha <= 0.0 {
            return;
        }
        if style.glow > 0.0 {
            // The glow is in canvas units, scaled like everything else on
            // this target (the 96×96 target's own K there).
            let sigma = style.glow * 0.5 * k_of::<T>();
            let r = box_radius(sigma);
            let region = d.grow_in(3 * r, T::WIDTH, T::HEIGHT);
            box_blur3(
                self.layer.cov.as_ref(),
                self.layer.blur.as_mut(),
                self.layer.tmp.as_mut(),
                T::WIDTH,
                region,
                r,
            );
            composite(self.fb, self.layer.blur.as_ref(), region, style);
        }
        composite(self.fb, self.layer.cov.as_ref(), d, style);
    }

    /// Add the lit pixels of a 1-bit bitmap to this draw call's coverage:
    /// `rows[y]` holds row `y` with bit 31 the left column, `w` columns
    /// wide, top-left at `(x, y)`, each bit `scale` units square. Sampled
    /// nearest, so in pixel units ([`Transform::in_pixels`]) at whole-pixel
    /// positions and quarter turns every bit lands exactly on one panel
    /// pixel, with no smoothing. Call between [`Self::begin`] and
    /// [`Self::finish`].
    pub fn cover_rows(&mut self, rows: &[u32], w: usize, x: f32, y: f32, scale: f32) {
        let h = rows.len();
        let r = self
            .xf
            .bounds(x, y, x + w as f32 * scale, y + h as f32 * scale, 0.0);
        for py in r.y0..r.y1 {
            for px in r.x0..r.x1 {
                let (cx, cy) = self.xf.inverse(px as f32 + 0.5, py as f32 + 0.5);
                let (bx, by) = (floorf((cx - x) / scale), floorf((cy - y) / scale));
                if bx < 0.0 || by < 0.0 || bx >= w as f32 || by >= h as f32 {
                    continue;
                }
                if rows[by as usize] & (0x8000_0000 >> bx as u32) != 0 {
                    self.cover(px, py, 1.0);
                }
            }
        }
        self.mark(r);
    }

    /// Rasterize coverage from a signed distance function over canvas-space
    /// bounds. `sdf` returns canvas units (negative inside).
    pub fn shape(&mut self, bounds: (f32, f32, f32, f32), style: Style, sdf: impl Fn(f32, f32) -> f32) {
        self.begin();
        let r = self.xf.bounds(bounds.0, bounds.1, bounds.2, bounds.3, 1.0);
        let ppu = self.xf.px_per_unit();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let (cx, cy) = self.xf.inverse(x as f32 + 0.5, y as f32 + 0.5);
                let d = sdf(cx, cy) * ppu;
                self.cover(x, y, (0.5 - d).clamp(0.0, 1.0));
            }
        }
        self.mark(r);
        self.finish(style);
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, style: Style) {
        if radius <= 0.0 {
            return;
        }
        self.shape(
            (cx - radius, cy - radius, cx + radius, cy + radius),
            style,
            |x, y| sqrtf((x - cx) * (x - cx) + (y - cy) * (y - cy)) - radius,
        );
    }

    /// Round-capped, round-joined stroke through `points`.
    pub fn stroke_polyline(&mut self, points: &[(f32, f32)], width: f32, style: Style) {
        self.stroke_paths(&[points], width, style);
    }

    /// Several open sub-paths stroked as one canvas `stroke()` call, so the
    /// glow and blending treat them as one shape.
    pub fn stroke_paths(&mut self, paths: &[&[(f32, f32)]], width: f32, style: Style) {
        let h = width / 2.0;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(x, y) in paths.iter().flat_map(|p| p.iter()) {
            x0 = x0.min(x - h);
            y0 = y0.min(y - h);
            x1 = x1.max(x + h);
            y1 = y1.max(y + h);
        }
        if x0 > x1 {
            return;
        }
        self.shape((x0, y0, x1, y1), style, |x, y| {
            let mut best = f32::MAX;
            for points in paths {
                if points.len() == 1 {
                    best = best.min(sqrtf(sq(x - points[0].0) + sq(y - points[0].1)));
                }
                for w in points.windows(2) {
                    best = best.min(segment_distance(x, y, w[0], w[1]));
                }
            }
            best - h
        });
    }

    /// Circular arc stroke with butt ends, from `start` to `end` radians
    /// (canvas angles: 0 = +x, clockwise on screen).
    #[allow(clippy::too_many_arguments)]
    pub fn stroke_arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        start: f32,
        end: f32,
        width: f32,
        style: Style,
    ) {
        let h = width / 2.0;
        let e = radius + h;
        self.shape((cx - e, cy - e, cx + e, cy + e), style, |x, y| {
            let (dx, dy) = (x - cx, y - cy);
            let ring = fabsf(sqrtf(dx * dx + dy * dy) - radius) - h;
            let a = libm::atan2f(dy, dx);
            let inside = angle_between(a, start, end);
            if inside {
                ring
            } else {
                // Distance to the nearer end cap (flat).
                let p0 = (cx + radius * cosf(start), cy + radius * sinf(start));
                let p1 = (cx + radius * cosf(end), cy + radius * sinf(end));
                let d0 = sqrtf(sq(x - p0.0) + sq(y - p0.1));
                let d1 = sqrtf(sq(x - p1.0) + sq(y - p1.1));
                d0.min(d1).max(ring)
            }
        });
    }

    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, style: Style) {
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        self.shape((x, y, x + w, y + h), style, |px, py| {
            box_distance(px - cx, py - cy, hx, hy)
        });
    }

    /// Filled triangle (the menu's ▲ and ▼, which the font doesn't have).
    pub fn fill_triangle(&mut self, p: [(f32, f32); 3], style: Style) {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (x, y) in p {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        self.shape((x0, y0, x1, y1), style, |x, y| triangle_distance(x, y, p));
    }

    /// Rectangle outline centred on the edge, like canvas `strokeRect`.
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, width: f32, style: Style) {
        let (cx, cy, hx, hy, lw) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0, width / 2.0);
        self.shape((x - lw, y - lw, x + w + lw, y + h + lw), style, |px, py| {
            fabsf(box_distance(px - cx, py - cy, hx, hy)) - lw
        });
    }
}

/// Signed distance to a triangle (negative inside).
pub fn triangle_distance(x: f32, y: f32, p: [(f32, f32); 3]) -> f32 {
    let mut d = f32::MAX;
    for i in 0..3 {
        d = d.min(segment_distance(x, y, p[i], p[(i + 1) % 3]));
    }
    let side = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
    let s = [side(p[0], p[1]), side(p[1], p[2]), side(p[2], p[0])];
    let inside = s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0);
    if inside {
        -d
    } else {
        d
    }
}

fn sq(v: f32) -> f32 {
    v * v
}

/// Distance from `(x, y)` to the segment `a`–`b`.
pub fn segment_distance(x: f32, y: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 0.0 {
        (((x - a.0) * vx + (y - a.1) * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (px, py) = (a.0 + vx * t - x, a.1 + vy * t - y);
    sqrtf(px * px + py * py)
}

/// Signed distance to an axis-aligned box of half-size `(hx, hy)`.
fn box_distance(x: f32, y: f32, hx: f32, hy: f32) -> f32 {
    let (dx, dy) = (fabsf(x) - hx, fabsf(y) - hy);
    let outside = sqrtf(sq(dx.max(0.0)) + sq(dy.max(0.0)));
    outside + dx.max(dy).min(0.0)
}

/// Is angle `a` on the clockwise sweep from `start` to `end`?
fn angle_between(a: f32, start: f32, end: f32) -> bool {
    let tau = core::f32::consts::TAU;
    let norm = |v: f32| ((v % tau) + tau) % tau;
    let sweep = norm(end - start);
    let rel = norm(a - start);
    rel <= sweep || sweep == 0.0 && end != start
}

/// Box radius for three passes approximating a Gaussian of `sigma` px.
fn box_radius(sigma: f32) -> usize {
    let w = sqrtf(12.0 * sigma * sigma / 3.0 + 1.0);
    roundf((w - 1.0) / 2.0).max(0.0) as usize
}

/// Three horizontal then three vertical box passes over `region` of planes
/// `width` pixels across.
fn box_blur3(src: &[u8], dst: &mut [u8], tmp: &mut [u8], width: usize, region: Rect, r: usize) {
    for y in region.y0..region.y1 {
        let row = y * width;
        dst[row + region.x0..row + region.x1].copy_from_slice(&src[row + region.x0..row + region.x1]);
    }
    if r == 0 {
        return;
    }
    // Three horizontal passes (dst → tmp → dst → tmp), then three vertical
    // (tmp → dst → tmp → dst): the result ends in `dst`.
    box_pass(dst, tmp, width, region, r, true);
    box_pass(tmp, dst, width, region, r, true);
    box_pass(dst, tmp, width, region, r, true);
    box_pass(tmp, dst, width, region, r, false);
    box_pass(dst, tmp, width, region, r, false);
    box_pass(tmp, dst, width, region, r, false);
}

/// One box pass (window 2r+1, zero outside the region) from `src` to `dst`.
fn box_pass(src: &[u8], dst: &mut [u8], width: usize, region: Rect, r: usize, horizontal: bool) {
    let w = (2 * r + 1) as u32;
    let (outer, inner) = if horizontal {
        ((region.y0, region.y1), (region.x0, region.x1))
    } else {
        ((region.x0, region.x1), (region.y0, region.y1))
    };
    if inner.0 >= inner.1 {
        return;
    }
    let idx = |o: usize, i: usize| {
        if horizontal {
            o * width + i
        } else {
            i * width + o
        }
    };
    for o in outer.0..outer.1 {
        // Window for i is [i - r, i + r], clipped to the region.
        let mut sum: u32 = 0;
        for j in inner.0..=(inner.0 + r).min(inner.1 - 1) {
            sum += src[idx(o, j)] as u32;
        }
        for i in inner.0..inner.1 {
            dst[idx(o, i)] = ((sum + w / 2) / w) as u8;
            if i + 1 + r < inner.1 {
                sum += src[idx(o, i + 1 + r)] as u32;
            }
            if i >= inner.0 + r {
                sum -= src[idx(o, i - r)] as u32;
            }
        }
    }
}

fn composite<T: Target>(fb: &mut Framebuffer<T>, cov: &[u8], region: Rect, style: Style) {
    let color = style.color.to_f32();
    for y in region.y0..region.y1 {
        for x in region.x0..region.x1 {
            let c = cov[y * T::WIDTH + x];
            if c == 0 {
                continue;
            }
            let a = c as f32 / 255.0 * style.alpha;
            fb.blend_rgb(x, y, color, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_round_trips_and_centres() {
        let xf = Transform::rotated(0.7).scaled(0.9).offset(10.0, -4.0);
        let (px, py) = xf.forward(12.0, -30.0);
        let (x, y) = xf.inverse(px, py);
        assert!((x - 12.0).abs() < 1e-3 && (y + 30.0).abs() < 1e-3);
        let (cx, cy) = Transform::default().forward(0.0, 0.0);
        assert_eq!((cx, cy), (48.0, 48.0));
        // The lit edge (83 canvas units) lands on the panel edge.
        assert!((Transform::default().forward(83.0, 0.0).0 - 96.0).abs() < 1e-3);
    }

    #[test]
    fn quarter_turn_matches_canvas_rotate() {
        // Canvas rotate(π/2): +x goes to +y (down) on screen.
        let (px, py) = Transform::quarter(Quarter::R90).forward(10.0, 0.0);
        assert!((px - 48.0).abs() < 1e-3 && (py - (48.0 + 10.0 * K)).abs() < 1e-3);
    }

    #[test]
    fn circle_is_antialiased_and_centred() {
        let mut fb = Framebuffer::<Grey96>::new();
        let mut layer = Layer::new();
        let mut p = Painter::new(&mut fb, &mut layer, Transform::default());
        p.fill_circle(0.0, 0.0, 12.45, Style::new(255, 1.0, 0.0));
        assert_eq!(fb.pixel(48, 48), 255);
        assert_eq!(fb.pixel(0, 0), 0);
        // Edge pixels are partial.
        assert!((0..96).any(|x| (1..255).contains(&fb.pixel(x, 48))));
    }

    #[test]
    fn glow_spreads_beyond_the_shape() {
        let draw = |glow| {
            let mut fb = Framebuffer::<Grey96>::new();
            let mut layer = Layer::new();
            Painter::new(&mut fb, &mut layer, Transform::default()).fill_circle(
                0.0,
                0.0,
                10.0,
                Style::new(255, 1.0, glow),
            );
            fb.pixel(48 + 12, 48)
        };
        assert_eq!(draw(0.0), 0);
        assert!(draw(18.0) > 0);
    }

    #[test]
    fn the_transform_follows_the_target() {
        use smokebomb_hal::Rgb64;
        // On both panels the lit edge (83 canvas units) is the panel edge.
        let xf = Transform::quarter_on::<Rgb64>(Quarter::R0);
        assert_eq!(xf.forward(0.0, 0.0), (32.0, 32.0));
        assert!((xf.forward(83.0, 0.0).0 - 64.0).abs() < 1e-3);
        assert_eq!(xf.panel(), (64, 64));
        // In pixel units one unit is one panel pixel.
        let px = xf.in_pixels();
        let (x, y) = px.forward(-32.0, 31.0);
        assert!((x - 0.0).abs() < 1e-4 && (y - 63.0).abs() < 1e-4);
        // Bounds clip to the target's panel, not the 96×96 one.
        let r = xf.bounds(-200.0, -200.0, 200.0, 200.0, 0.0);
        assert_eq!((r.x1, r.y1), (64, 64));
    }

    #[test]
    fn primitives_draw_on_both_targets() {
        use smokebomb_hal::{Color, Pixel, Rgb565, Rgb64};
        let gold = Color::hex(0xF5C451);
        // A filled circle and a stroke in colour, on the 64×64 target.
        let mut fb = Framebuffer::<Rgb64>::new();
        let mut layer = Layer::<Rgb64>::new();
        let mut p = Painter::new(&mut fb, &mut layer, Transform::quarter_on::<Rgb64>(Quarter::R0));
        p.fill_circle(0.0, 0.0, 20.0, Style::color(gold, 1.0, 0.0));
        p.stroke_polyline(&[(-60.0, 60.0), (60.0, 60.0)], 6.0, Style::new(255, 1.0, 0.0));
        p.fill_rect(
            -80.0,
            -80.0,
            10.0,
            10.0,
            Style::color(Color::rgb(0, 0, 255), 1.0, 0.0),
        );
        assert_eq!(fb.pixel(32, 32), Rgb565::from_color(gold));
        assert_eq!(fb.pixel(32, 55).color(), Color::WHITE);
        assert_eq!(fb.pixel(2, 2), Rgb565::from_rgb(0, 0, 255));
        assert_eq!(fb.pixel(63, 0), Rgb565::BLACK);
        // The same calls on the 96×96 target give grey: the colour's
        // brightest channel, as the mockup's quantiser reads it.
        let mut g = Framebuffer::<Grey96>::new();
        let mut gl = Layer::<Grey96>::new();
        let mut q = Painter::new(&mut g, &mut gl, Transform::quarter(Quarter::R0));
        q.fill_circle(0.0, 0.0, 20.0, Style::color(gold, 1.0, 0.0));
        assert_eq!(g.pixel(48, 48), 0xF5);
    }

    #[test]
    fn bitmaps_land_on_whole_pixels_at_every_quarter_turn() {
        use smokebomb_hal::{Pixel, Rgb64};
        // An L: the top row and the left column of a 3×3 box.
        let rows = [0b111u32 << 29, 0b100 << 29, 0b100 << 29];
        for q in [Quarter::R0, Quarter::R90, Quarter::R180, Quarter::R270] {
            let mut fb = Framebuffer::<Rgb64>::new();
            let mut layer = Layer::<Rgb64>::new();
            let xf = Transform::quarter_on::<Rgb64>(q).in_pixels();
            let mut p = Painter::new(&mut fb, &mut layer, xf);
            p.begin();
            p.cover_rows(&rows, 3, 4.0, 4.0, 1.0);
            p.finish(Style::new(255, 1.0, 0.0));
            let lit: usize = fb.pixels().iter().filter(|px| px.level() == 255).count();
            let partial = fb
                .pixels()
                .iter()
                .filter(|px| (1..255).contains(&px.level()))
                .count();
            assert_eq!((lit, partial), (5, 0), "{q:?}: every bit is one whole pixel");
            // Content pixel (4 + 32, 4 + 32) is the L's corner, wherever the
            // quarter turn puts it.
            let (x, y) = q.map_in(36, 36, 64);
            assert_eq!(fb.pixel(x, y).level(), 255, "{q:?}");
        }
    }

    #[test]
    fn box_blur_preserves_mass() {
        use crate::display::PIXELS;
        let mut src = [0u8; PIXELS];
        src[48 * 96 + 48] = 255;
        let mut dst = [0u8; PIXELS];
        let mut tmp = [0u8; PIXELS];
        box_blur3(
            &src,
            &mut dst,
            &mut tmp,
            96,
            Rect {
                x0: 0,
                y0: 0,
                x1: 96,
                y1: 96,
            },
            1,
        );
        let total: u32 = dst.iter().map(|&v| v as u32).sum();
        assert!(total > 150 && total < 360, "{total}");
    }
}
