//! The 30 mm die's 64×64 screens next to the 34 mm die's 96×96 ones.
//!
//! Every screen is drawn by the firmware's own code: the 64×64 layouts
//! (`screens64`) on the `Rgb64` target and their 96×96 counterparts
//! (`screens`) on `Grey96`, quantised to the panel's 16 levels as it shows
//! them. The roll animation comes from the whole firmware running a throw on
//! the simulator, once per target, with the same seeds.
//!
//! Two outputs:
//!
//! * `tests/snapshots/rgb64-screens.png`: every 64×64 screen at 1:1 in a
//!   strip, compared exactly. Regenerate with `UPDATE_SNAPSHOTS=1` when a
//!   change is intended, like the 96×96 sheets.
//! * `docs/30mm/contact-sheet-1.png` and `-2.png` (written with
//!   `UPDATE_SNAPSHOTS=1`, and always to `target/`): per screen, the 64×64
//!   at 1:1 and 8×, beside the 96×96 at 1:1 and 5×, with the glass's rounded
//!   mask shaded on the enlarged tiles. Two pages, so each stays under
//!   8000 px tall.

use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};

use heapless::Vec as HVec;
use smokebomb_core::display::Framebuffer;
use smokebomb_core::font::Fonts;
use smokebomb_core::font64::{Glyph, TEXT};
use smokebomb_core::gfx::{Layer, Painter, Transform};
use smokebomb_core::menu::{Draft, Page, Settings, Setup};
use smokebomb_core::nest::{ChargeView, Label, NestFace, Screen};
use smokebomb_core::orientation::Quarter;
use smokebomb_core::pack::PackIndex;
use smokebomb_core::screens::{self, Ctx, LOOP_END};
use smokebomb_core::screens64;
use smokebomb_core::smoke::Special;
use smokebomb_core::target::DisplayTarget;
use smokebomb_core::Firmware;
use smokebomb_hal::{Color, Face, Grey96, Pixel, Rgb64, Target};
use smokebomb_hal_simulator::{imu_script, SimAssets, SimHandle, SimPlatform};
use smokebomb_shared::types::{DeviceSerial, SessionId};
use smokebomb_shared::{DieKind, RollRecord};

type Rgb = [u8; 3];

/// A face as the panel shows it, `side` pixels square.
struct Tile {
    side: usize,
    px: Vec<Rgb>,
}

fn tile_of_grey(fb: &Framebuffer<Grey96>) -> Tile {
    let mut packed = [0u8; smokebomb_hal::FRAME_BYTES];
    fb.quantize(&mut packed);
    let px = (0..96 * 96)
        .map(|i| {
            let b = packed[i / 2];
            let l = if i % 2 == 0 { b >> 4 } else { b & 15 } * 17;
            [l, l, l]
        })
        .collect();
    Tile { side: 96, px }
}

fn tile_of_rgb(fb: &Framebuffer<Rgb64>) -> Tile {
    let px = fb
        .pixels()
        .iter()
        .map(|p| {
            let c = p.color();
            [c.r, c.g, c.b]
        })
        .collect();
    Tile { side: 64, px }
}

/// Fonts and the asset pack, shared by every drawing.
struct Kit {
    assets: SimAssets,
    fonts: Box<MaybeUninit<Fonts>>,
}

impl Kit {
    fn new() -> Self {
        let sim = SimHandle::new();
        let mut assets = sim.peripherals().assets;
        let pack = PackIndex::load(&mut assets);
        let mut fonts = Box::new(MaybeUninit::uninit());
        Fonts::init(&mut fonts, &mut assets, &pack);
        Self { assets, fonts }
    }

    fn draw<T: DisplayTarget>(&mut self, draw: &dyn Fn(&mut Ctx<SimAssets, T>)) -> Framebuffer<T> {
        let mut fb = Framebuffer::<T>::new();
        let mut layer = Box::new(Layer::<T>::new());
        let mut painter = Painter::new(&mut fb, &mut layer, Transform::quarter_on::<T>(Quarter::R0));
        // SAFETY: `Fonts::init` filled it in `new`.
        let fonts = unsafe { self.fonts.assume_init_mut() };
        let mut c = Ctx {
            painter: &mut painter,
            fonts,
            assets: &mut self.assets,
        };
        draw(&mut c);
        fb
    }
}

fn record(die: DieKind, values: &[u8]) -> RollRecord {
    RollRecord {
        device: DeviceSerial([0; 9]),
        session: SessionId([0; 16]),
        counter: 1,
        uptime_ms: 0,
        die,
        values: HVec::from_slice(values).unwrap(),
        prev_hash: [0; 32],
    }
}

fn draft(page: Page, die: DieKind, count: u8) -> Draft {
    let mut d = Draft::new(&Settings::default());
    d.page = page;
    d.die = die;
    d.count = count;
    d
}

/// A page of the Settings app.
fn setting(page: Page) -> Draft {
    let mut d = draft(page, DieKind::D20, 1);
    d.in_settings = true;
    d
}

fn charge(pct: f32, label: Label) -> NestFace {
    NestFace {
        screen: Screen::Charge(ChargeView {
            fill: pct / 100.0,
            pct,
            label,
            alpha: 1.0,
            text: 1.0,
            flash: 0.0,
            wave: 0.6,
        }),
        dim: 1.0,
        shift: (0.0, 0.0),
    }
}

/// The firmware mid-throw on target `T`: the up face 0.5 s after release.
fn throw_frame<T: DisplayTarget>() -> Vec<u8> {
    let sim = SimHandle::new();
    {
        let mut s = sim.lock();
        s.manual_time_ms = Some(0);
        s.imu_resting = imu_script::resting(Face::PosY);
    }
    let mut fw = Firmware::new(sim.peripherals_for::<T>()).unwrap();
    if let Some(smoke) = fw.smoke_mut() {
        smoke.set_rng(smokebomb_core::smoke::SmokeRng::new(42));
    }
    let mut t = 0u64;
    let mut run = |fw: &mut Firmware<SimPlatform<T>>, ticks: u64| {
        for _ in 0..ticks {
            t += 17;
            sim.lock().manual_time_ms = Some(t);
            fw.tick().unwrap();
        }
    };
    run(&mut fw, 7 * 60);
    sim.lock().imu_script.extend(imu_script::throw());
    run(&mut fw, 60);
    let face = sim.lock().faces[Face::PosY.index()].clone();
    face
}

fn throw_tiles() -> (Tile, Tile) {
    let grey = throw_frame::<Grey96>();
    let mut fb = Framebuffer::<Grey96>::new();
    fb.load_packed(grey.as_slice().try_into().unwrap());
    let rgb = throw_frame::<Rgb64>();
    let mut fb64 = Framebuffer::<Rgb64>::new();
    fb64.load_packed565(rgb.as_slice().try_into().unwrap());
    (tile_of_grey(&fb), tile_of_rgb(&fb64))
}

type Draw96 = Box<dyn Fn(&mut Ctx<SimAssets, Grey96>)>;
type Draw64 = Box<dyn Fn(&mut Ctx<SimAssets, Rgb64>)>;

/// Each row: a name, the 96×96 counterpart (or none) and the 64×64 screen.
fn rows() -> Vec<(&'static str, Option<Draw96>, Draw64)> {
    let d20 = Setup::Roll(DieKind::D20, 1);
    let three = Setup::Roll(DieKind::D6, 3);
    vec![
        (
            "boot: pips",
            Some(Box::new(|c| screens::draw_boot(c, 0, false, 1.0))),
            Box::new(|c| screens64::draw_boot(c, 0, false, 1.0)),
        ),
        (
            "boot: logo",
            Some(Box::new(|c| screens::draw_boot(c, 2, true, LOOP_END + 2.5))),
            Box::new(|c| screens64::draw_boot(c, 2, true, LOOP_END + 2.5)),
        ),
        (
            "idle: d20, face 3",
            Some(Box::new(move |c| screens::draw_wake_label(c, d20, "d20", 1.0))),
            Box::new(move |c| screens64::draw_idle(c, d20, "d20", 1.0, 0)),
        ),
        (
            "idle: 3d6, face 1",
            Some(Box::new(move |c| screens::draw_wake_label(c, three, "3d6", 1.0))),
            Box::new(move |c| screens64::draw_idle(c, three, "3d6", 1.0, 2)),
        ),
        (
            "result: d20 17",
            Some(Box::new(|c| {
                screens::draw_result(c, &record(DieKind::D20, &[17]), None, 1.0)
            })),
            Box::new(|c| screens64::draw_result(c, &record(DieKind::D20, &[17]), None, 1.0)),
        ),
        (
            "result: max (crit)",
            Some(Box::new(|c| {
                screens::draw_result(c, &record(DieKind::D20, &[20]), Some(Special::Max), 1.0)
            })),
            Box::new(|c| screens64::draw_result(c, &record(DieKind::D20, &[20]), Some(Special::Max), 1.0)),
        ),
        (
            "result: min (fumble)",
            Some(Box::new(|c| {
                screens::draw_result(c, &record(DieKind::D20, &[1]), Some(Special::Dud), 1.0)
            })),
            Box::new(|c| screens64::draw_result(c, &record(DieKind::D20, &[1]), Some(Special::Dud), 1.0)),
        ),
        (
            "result: 3d6 pool",
            Some(Box::new(|c| {
                screens::draw_result(c, &record(DieKind::D6, &[3, 5, 2]), None, 1.0)
            })),
            Box::new(|c| screens64::draw_result(c, &record(DieKind::D6, &[3, 5, 2]), None, 1.0)),
        ),
        (
            "result: d100",
            Some(Box::new(|c| {
                screens::draw_result(c, &record(DieKind::D100, &[100]), None, 1.0)
            })),
            Box::new(|c| screens64::draw_result(c, &record(DieKind::D100, &[100]), None, 1.0)),
        ),
        (
            "result: 10d20",
            Some(Box::new(|c| {
                screens::draw_result(
                    c,
                    &record(DieKind::D20, &[12, 7, 19, 3, 15, 8, 20, 11, 6, 14]),
                    None,
                    1.0,
                )
            })),
            Box::new(|c| {
                screens64::draw_result(
                    c,
                    &record(DieKind::D20, &[12, 7, 19, 3, 15, 8, 20, 11, 6, 14]),
                    None,
                    1.0,
                )
            }),
        ),
        (
            "result: 10d100",
            Some(Box::new(|c| {
                screens::draw_result(
                    c,
                    &record(DieKind::D100, &[100, 99, 98, 97, 96, 95, 94, 93, 92, 91]),
                    None,
                    1.0,
                )
            })),
            Box::new(|c| {
                screens64::draw_result(
                    c,
                    &record(DieKind::D100, &[100, 99, 98, 97, 96, 95, 94, 93, 92, 91]),
                    None,
                    1.0,
                )
            }),
        ),
        (
            "menu: which die",
            Some(Box::new(|c| {
                screens::draw_menu(c, &draft(Page::Die, DieKind::D20, 1), 0.78, 0.0, 0.0, 1.0, 1.0)
            })),
            Box::new(|c| {
                screens64::draw_menu(c, &draft(Page::Die, DieKind::D20, 1), 0.78, 0.0, 0.0, 1.0, 1.0)
            }),
        ),
        (
            "menu: how many",
            Some(Box::new(|c| {
                screens::draw_menu(c, &draft(Page::Count, DieKind::D6, 3), 0.78, 0.0, 0.0, 1.0, 1.0)
            })),
            Box::new(|c| {
                screens64::draw_menu(c, &draft(Page::Count, DieKind::D6, 3), 0.78, 0.0, 0.0, 1.0, 1.0)
            }),
        ),
        (
            "menu: settings",
            Some(Box::new(|c| {
                screens::draw_menu(c, &setting(Page::Setting(0)), 0.78, 0.0, 0.0, 1.0, 1.0)
            })),
            Box::new(|c| screens64::draw_menu(c, &setting(Page::Setting(0)), 0.78, 0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "menu: power off",
            Some(Box::new(|c| {
                screens::draw_menu(c, &setting(Page::Power), 0.78, 0.0, 0.0, 1.0, 1.0)
            })),
            Box::new(|c| screens64::draw_menu(c, &setting(Page::Power), 0.78, 0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "menu: legal",
            Some(Box::new(|c| {
                screens::draw_menu(c, &setting(Page::Legal), 0.12, 0.0, 0.0, 1.0, 1.0)
            })),
            Box::new(|c| screens64::draw_menu(c, &setting(Page::Legal), 0.12, 0.0, 0.0, 1.0, 1.0)),
        ),
        (
            "saved",
            Some(Box::new(|c| {
                screens::draw_success(c, "d20", "Ready to roll", 0.6)
            })),
            Box::new(|c| screens64::draw_success(c, "d20", "Ready to roll", 0.6)),
        ),
        (
            "text: 1d20 + 5 = 17",
            None,
            Box::new(|c| screens64::draw_modifier(c, "1d20", 12, 5, 1.0)),
        ),
        (
            "text: 2d6 - 1 = 6",
            None,
            Box::new(|c| screens64::draw_modifier(c, "2d6", 7, -1, 1.0)),
        ),
        (
            "text: paragraph",
            None,
            Box::new(|c| {
                screens64::draw_text_block(
                    c,
                    "Hold a face for the menu. Tip to pick, hold to save.",
                    smokebomb_core::palette64::WHITE,
                    1.0,
                );
            }),
        ),
        (
            "charging",
            Some(Box::new(|c| {
                screens::draw_nest(c, &charge(62.0, Label::Charging))
            })),
            Box::new(|c| {
                screens64::draw_nest(c, &charge(62.0, Label::Charging));
            }),
        ),
        (
            "battery low",
            Some(Box::new(|c| screens::draw_bolt(c, 1.0))),
            Box::new(|c| screens64::draw_low_battery(c, 1.0)),
        ),
    ]
}

// ---------- the sheet ----------

struct Sheet {
    w: usize,
    h: usize,
    px: Vec<Rgb>,
}

impl Sheet {
    fn new(w: usize, h: usize, bg: Rgb) -> Self {
        Self {
            w,
            h,
            px: vec![bg; w * h],
        }
    }

    fn put(&mut self, x: usize, y: usize, c: Rgb) {
        if x < self.w && y < self.h {
            self.px[y * self.w + x] = c;
        }
    }

    /// `tile` at `scale`× nearest-neighbour, top-left at (x, y); with
    /// `mask`, the glass's rounded corner (radius `r` panel px) shades what
    /// it hides.
    fn tile(&mut self, t: &Tile, x: usize, y: usize, scale: usize, mask: Option<f32>) {
        let side = (t.side * scale) as f32;
        for py in 0..t.side * scale {
            for px in 0..t.side * scale {
                let mut c = t.px[(py / scale) * t.side + px / scale];
                if let Some(r) = mask {
                    let r = r * scale as f32;
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    let dx = (r - fx).max(fx - (side - r)).max(0.0);
                    let dy = (r - fy).max(fy - (side - r)).max(0.0);
                    if dx * dx + dy * dy > r * r {
                        c = [c[0] / 5 + 22, c[1] / 5 + 22, c[2] / 5 + 24];
                    }
                }
                self.put(x + px, y + py, c);
            }
        }
    }

    fn text(&mut self, s: &str, x: usize, y: usize, scale: usize, c: Rgb) {
        let mut pen = x;
        for ch in s.chars() {
            let Some(g): Option<&Glyph<9>> = TEXT.find(ch) else {
                continue;
            };
            for (row, bits) in g.rows.iter().enumerate() {
                for col in 0..g.width as usize {
                    if bits & (0x8000_0000 >> col) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                self.put(pen + col * scale + dx, y + row * scale + dy, c);
                            }
                        }
                    }
                }
            }
            pen += (g.width as usize + 1) * scale;
        }
    }

    fn write(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        let file = std::fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        let data: Vec<u8> = self.px.iter().flatten().copied().collect();
        w.write_image_data(&data).unwrap();
    }
}

fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let dec = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width as usize, info.height as usize, buf))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

const INK: Rgb = [0xE9, 0xEB, 0xEE];
const SOFT: Rgb = [0x8A, 0x8C, 0x90];
const BG: Rgb = [0x16, 0x17, 0x1A];

#[test]
fn contact_sheet() {
    let mut kit = Kit::new();
    let (throw96, throw64) = throw_tiles();
    let mut tiles: Vec<(&str, Option<Tile>, Tile)> = vec![("roll: mid-throw", Some(throw96), throw64)];
    for (name, d96, d64) in rows() {
        let t96 = d96.map(|d| tile_of_grey(&kit.draw::<Grey96>(&*d)));
        let t64 = tile_of_rgb(&kit.draw::<Rgb64>(&*d64));
        tiles.push((name, t96, t64));
    }
    // Keep the boot rows first, as the firmware plays them.
    tiles.rotate_left(1);
    let throw = tiles.pop().unwrap();
    tiles.insert(4, throw);

    // The 64×64 strip, 1:1, compared exactly.
    let mut strip = Sheet::new(tiles.len() * 66, 64, [0, 0, 0]);
    for (i, (_, _, t)) in tiles.iter().enumerate() {
        strip.tile(t, i * 66, 0, 1, None);
    }
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/rgb64-screens.png");
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();
    if update {
        strip.write(&golden);
    } else {
        let (w, h, data) = read_png(&golden).expect("no rgb64-screens.png; run with UPDATE_SNAPSHOTS=1");
        let ours: Vec<u8> = strip.px.iter().flatten().copied().collect();
        if (w, h) != (strip.w, strip.h) || data != ours {
            strip.write(&repo_root().join("target/snapshot-diffs/rgb64-screens.png"));
            panic!("the 64×64 screens changed: see target/snapshot-diffs/rgb64-screens.png");
        }
    }

    // The contact sheet.
    const PAD: usize = 24;
    const LABEL_W: usize = 230;
    let (s64, s96) = (8, 5);
    let row_h = 64 * s64 + PAD;
    let x_64_1 = LABEL_W;
    let x_96_1 = x_64_1 + 64 + PAD;
    let x_64_8 = x_96_1 + 96 + PAD;
    let x_96_5 = x_64_8 + 64 * s64 + PAD;
    let width = x_96_5 + 96 * s96 + PAD;
    let header = 96;
    let per_page = tiles.len().div_ceil(2);
    for (page, chunk) in tiles.chunks(per_page).enumerate() {
        let mut sheet = Sheet::new(width, header + chunk.len() * row_h, BG);
        let title = format!(
            "Sugarcube 30 mm: 64x64 RGB565 vs 34 mm: 96x96 grey  ({}/2)",
            page + 1
        );
        sheet.text(&title, PAD, 20, 3, INK);
        sheet.text(
            "64x64 at 1:1 | 96x96 at 1:1 | 64x64 at 8x | 96x96 at 5x (same shown size). Shaded corners: the glass mask.",
            PAD,
            60,
            2,
            SOFT,
        );
        for (i, (name, t96, t64)) in chunk.iter().enumerate() {
            let y = header + i * row_h;
            sheet.text(name, PAD, y + 8, 2, INK);
            sheet.tile(t64, x_64_1, y, 1, None);
            sheet.tile(t64, x_64_8, y, s64, Some(Rgb64::MASK_RADIUS_PX));
            match t96 {
                Some(t) => {
                    sheet.tile(t, x_96_1, y, 1, None);
                    sheet.tile(t, x_96_5, y, s96, Some(Grey96::MASK_RADIUS_PX));
                }
                None => sheet.text("no 96x96 screen", x_96_5 + 24, y + 24, 2, SOFT),
            }
        }
        assert!(sheet.h < 8000, "page {} is {} px tall", page + 1, sheet.h);
        let name = format!("contact-sheet-{}.png", page + 1);
        sheet.write(&repo_root().join("target").join(&name));
        if update {
            sheet.write(&repo_root().join("docs/30mm").join(&name));
        }
    }
}

/// Nothing a 64×64 screen draws reaches past the panel's edge columns
/// where the glass hides it: every lit pixel of every screen sits inside
/// the rounded lit area.
#[test]
fn screens_stay_inside_the_rounded_lit_area() {
    let mut kit = Kit::new();
    for (name, _, d64) in rows() {
        // The charge fill is a background wash that runs under the mask on
        // purpose, as on the 96×96 die.
        if name == "charging" {
            continue;
        }
        let fb = kit.draw::<Rgb64>(&*d64);
        let r = Rgb64::MASK_RADIUS_PX;
        for y in 0..64 {
            for x in 0..64 {
                if fb.pixel(x, y).color() == Color::BLACK {
                    continue;
                }
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dx = (r - fx).max(fx - (64.0 - r)).max(0.0);
                let dy = (r - fy).max(fy - (64.0 - r)).max(0.0);
                assert!(
                    dx * dx + dy * dy <= r * r,
                    "{name}: pixel ({x}, {y}) is under the glass's corner"
                );
            }
        }
    }
}

/// The game screens drawn only on the 64×64 die (Hot Potato, Pass the Pot,
/// the hold ring): their sheet is `games64`, from whole games. Here, each
/// at the moments that reach furthest out.
fn game_screens64() -> Vec<(String, Draw64)> {
    let mut v: Vec<(String, Draw64)> = Vec::new();
    for n in 1..=3u8 {
        v.push((
            format!("bills {n}"),
            Box::new(move |c| screens64::draw_bills(c, n, 1.0)),
        ));
    }
    v.push((
        "potato label".into(),
        Box::new(|c| screens64::draw_potato_label(c, 1.0)),
    ));
    for heat in [0.0, 0.5, 1.0] {
        v.push((
            format!("fuse {heat}"),
            Box::new(move |c| screens64::draw_fuse(c, heat, 1.0, 0.37)),
        ));
    }
    // From 1.3 s on: the chunks before that fly past the corners, as the
    // smoke does.
    for t in [1.3, 2.0, 4.0] {
        v.push((format!("boom {t}"), Box::new(move |c| screens64::draw_boom(c, t))));
    }
    // Every Pass the Pot throw of up to three dice (raw 1 ←, 2 pot, 3 →, 4 keep).
    for n in 1..=3usize {
        for k in 0..4usize.pow(n as u32) {
            let values: Vec<u8> = (0..n).map(|i| (k / 4usize.pow(i as u32) % 4) as u8 + 1).collect();
            v.push((
                format!("pot {values:?}"),
                Box::new(move |c| {
                    screens64::draw_result(c, &record(DieKind::PassThePot, &values), None, 1.0)
                }),
            ));
        }
    }
    for (p, grow) in [(0.5, 0.0), (1.0, 0.0), (1.0, 3.0)] {
        v.push((
            format!("hold ring {p} {grow}"),
            Box::new(move |c| screens64::draw_hold_ring(c, p, 1.0, grow)),
        ));
    }
    v
}

#[test]
fn game_screens_stay_inside_the_rounded_lit_area() {
    let mut kit = Kit::new();
    for (name, d64) in game_screens64() {
        let fb = kit.draw::<Rgb64>(&*d64);
        let r = Rgb64::MASK_RADIUS_PX;
        let mut lit = 0;
        for y in 0..64 {
            for x in 0..64 {
                if fb.pixel(x, y).color() == Color::BLACK {
                    continue;
                }
                lit += 1;
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dx = (r - fx).max(fx - (64.0 - r)).max(0.0);
                let dy = (r - fy).max(fy - (64.0 - r)).max(0.0);
                assert!(
                    dx * dx + dy * dy <= r * r,
                    "{name}: pixel ({x}, {y}) is under the glass's corner"
                );
            }
        }
        assert!(lit > 0, "{name} drew nothing");
    }
}
