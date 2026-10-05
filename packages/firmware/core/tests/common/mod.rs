//! The harness the 64×64 game tests share: whole-firmware games on the
//! simulator, once per target, with the same scripted inputs, and the
//! sheets they draw. Each test file uses part of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use smokebomb_core::display::Framebuffer;
use smokebomb_core::font64::{Glyph, TEXT};
use smokebomb_core::menu::Settings;
use smokebomb_core::pigs::Pose;
use smokebomb_core::smoke::SmokeRng;
use smokebomb_core::target::DisplayTarget;
use smokebomb_core::Firmware;
use smokebomb_hal::{Face, Grey96, Pixel, Rgb64, Target};
use smokebomb_hal_simulator::{imu_script, SimHandle, SimPlatform};

pub const FPS: f64 = 60.0;

pub type Rgb = [u8; 3];

pub struct Run<T: DisplayTarget> {
    pub sim: SimHandle,
    pub fw: Box<Firmware<SimPlatform<T>>>,
    frame: u64,
}

impl<T: DisplayTarget> Run<T> {
    pub fn new(settings: Settings) -> Self {
        let sim = SimHandle::new();
        {
            let mut s = sim.lock();
            s.manual_time_ms = Some(0);
            s.imu_resting = imu_script::resting(Face::PosY);
            s.battery_percent = 78;
        }
        let mut fw = Box::new(Firmware::new(sim.peripherals_for::<T>()).unwrap());
        if let Some(smoke) = fw.smoke_mut() {
            smoke.set_rng(SmokeRng::new(42));
        }
        fw.set_settings(settings);
        let mut run = Self { sim, fw, frame: 0 };
        run.step();
        run
    }

    pub fn step(&mut self) {
        self.sim.lock().manual_time_ms = Some((self.frame as f64 * 1000.0 / FPS).round() as u64);
        self.fw.tick().unwrap();
        self.frame += 1;
    }

    pub fn advance_to(&mut self, t: f64) {
        let target = (t * FPS).round() as u64;
        while self.frame <= target {
            self.step();
        }
    }

    /// A throw landing +Z up, the random words after it scripted.
    pub fn throw_with(&mut self, words: &[u32]) {
        let mut s = self.sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosZ);
        s.rng_script.extend(words.iter().copied());
    }

    /// A throw landing +Z up with the pigs in `poses`, touching if asked.
    pub fn throw_pigs(&mut self, poses: [Pose; 2], touching: bool) {
        let mut words: Vec<u32> = poses.iter().map(|&p| pose_word(p)).collect();
        words.push(if touching { 0 } else { u32::MAX });
        self.throw_with(&words);
    }

    /// A shake in the hand, then put down +Z up.
    pub fn shake(&mut self) {
        let mut s = self.sim.lock();
        s.imu_script.extend(imu_script::shake());
        s.imu_resting = imu_script::resting(Face::PosZ);
    }

    pub fn tap(&mut self) {
        self.press();
        self.step();
        self.release();
    }

    /// A finger on the +Z screen, until [`Self::release`].
    pub fn press(&mut self) {
        self.sim.lock().touch_mask = 1 << Face::PosZ.index();
    }

    pub fn release(&mut self) {
        self.sim.lock().touch_mask = 0;
    }

    /// The +Z face as the panel shows it.
    pub fn face(&self) -> Tile {
        let bytes = self.sim.lock().faces[Face::PosZ.index()].clone();
        tile::<T>(&bytes)
    }
}

/// A random word that lands the pigs on `pose` (as the snapshot tests).
pub fn pose_word(pose: Pose) -> u32 {
    use smokebomb_core::pigs::WEIGHT_TOTAL;
    let start: u32 = Pose::ALL[..pose.index()].iter().map(|p| p.weight() as u32).sum();
    let mid = start * 2 + pose.weight() as u32;
    ((mid as u64 * (u32::MAX as u64 + 1)) / (2 * WEIGHT_TOTAL as u64)) as u32
}

pub struct Tile {
    pub side: usize,
    pub px: Vec<Rgb>,
}

pub fn tile<T: Target>(bytes: &[u8]) -> Tile {
    if T::WIDTH == 96 {
        let mut fb = Framebuffer::<Grey96>::new();
        fb.load_packed(bytes.try_into().unwrap());
        Tile {
            side: 96,
            px: fb.pixels().iter().map(|&l| [l, l, l]).collect(),
        }
    } else {
        let mut fb = Framebuffer::<Rgb64>::new();
        fb.load_packed565(bytes.try_into().unwrap());
        Tile {
            side: 64,
            px: fb
                .pixels()
                .iter()
                .map(|p| {
                    let c = p.color();
                    [c.r, c.g, c.b]
                })
                .collect(),
        }
    }
}

pub type Input<T> = fn(&mut Run<T>);

/// A scenario: its name, the settings it starts from, its checkpoint times,
/// and its inputs for each target.
pub struct Scenario {
    pub name: &'static str,
    pub settings: fn() -> Settings,
    pub times: &'static [f64],
    pub grey: Vec<(f64, Input<Grey96>)>,
    pub rgb: Vec<(f64, Input<Rgb64>)>,
}

#[macro_export]
macro_rules! scenario {
    ($name:expr, $settings:expr, $times:expr, [$(($t:expr, $f:expr)),* $(,)?]) => {
        common::Scenario {
            name: $name,
            settings: $settings,
            times: $times,
            grey: vec![$(($t, $f as common::Input<smokebomb_hal::Grey96>)),*],
            rgb: vec![$(($t, $f as common::Input<smokebomb_hal::Rgb64>)),*],
        }
    };
}

/// Play `inputs` from `settings`, taking the +Z face at each of `times`.
pub fn play<T: DisplayTarget>(settings: Settings, times: &[f64], inputs: &[(f64, Input<T>)]) -> Vec<Tile> {
    let mut run = Run::<T>::new(settings);
    let mut inputs = inputs.to_vec();
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut next = 0;
    for &t in times {
        while next < inputs.len() && inputs[next].0 <= t {
            run.advance_to(inputs[next].0);
            (inputs[next].1)(&mut run);
            next += 1;
        }
        run.advance_to(t);
        out.push(run.face());
    }
    out
}

/// A scenario played on both panels: its name, checkpoint times, and the
/// 96×96 and 64×64 tiles.
pub type Played = (&'static str, &'static [f64], Vec<Tile>, Vec<Tile>);

pub fn play_both(scenarios: Vec<Scenario>) -> Vec<Played> {
    scenarios
        .into_iter()
        .map(|s| {
            (
                s.name,
                s.times,
                play::<Grey96>((s.settings)(), s.times, &s.grey),
                play::<Rgb64>((s.settings)(), s.times, &s.rgb),
            )
        })
        .collect()
}

/// Compare every 64×64 checkpoint, at 1:1 with a row per scenario, against
/// `tests/snapshots/{golden}.png` exactly, and draw the comparison sheet:
/// per scenario, 64×64 at 4× over 96×96 at 3×. The sheet always goes to
/// `target/{sheet}.png`, and with `UPDATE_SNAPSHOTS` (which also rewrites
/// the golden) to `docs/30mm/{sheet}.png`.
pub fn check_and_draw(played: &[Played], golden: &str, sheet_name: &str, title: &str) {
    let cols = played.iter().map(|p| p.1.len()).max().unwrap();
    let mut strip = Sheet::new(cols * 66, played.len() * 66, [0, 0, 0]);
    for (r, (_, _, _, rgb)) in played.iter().enumerate() {
        for (c, t) in rgb.iter().enumerate() {
            strip.tile(t, c * 66, r * 66, 1);
        }
    }
    let golden_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/snapshots/{golden}.png"));
    let update = std::env::var_os("UPDATE_SNAPSHOTS").is_some();

    const PAD: usize = 16;
    let cell = 288 + PAD;
    let block = 40 + 256 + 8 + 288 + PAD * 2;
    let mut sheet = Sheet::new(PAD + cols * cell, 90 + played.len() * block, [0x16, 0x17, 0x1A]);
    sheet.text(title, PAD, 20, 3, [0xE9, 0xEB, 0xEE]);
    sheet.text(
        "Whole-firmware games, the up face at each checkpoint (seconds since boot).",
        PAD,
        56,
        2,
        [0x8A, 0x8C, 0x90],
    );
    for (r, (name, times, grey, rgb)) in played.iter().enumerate() {
        let y = 90 + r * block;
        sheet.text(name, PAD, y + 8, 2, [0xE9, 0xEB, 0xEE]);
        for (c, ((t64, t96), at)) in rgb.iter().zip(grey).zip(times.iter()).enumerate() {
            let x = PAD + c * cell;
            sheet.text(&format!("{at:.1} s"), x + 200, y + 8, 2, [0x8A, 0x8C, 0x90]);
            sheet.tile(t64, x + 16, y + 40, 4);
            sheet.tile(t96, x, y + 40 + 256 + 8, 3);
        }
    }
    sheet.write(&repo_root().join(format!("target/{sheet_name}.png")));
    if update {
        strip.write(&golden_path);
        sheet.write(&repo_root().join(format!("docs/30mm/{sheet_name}.png")));
    } else {
        let (w, h, data) =
            read_png(&golden_path).unwrap_or_else(|| panic!("no {golden}.png; run with UPDATE_SNAPSHOTS=1"));
        let ours: Vec<u8> = strip.px.iter().flatten().copied().collect();
        if (w, h) != (strip.w, strip.h) || data != ours {
            strip.write(&repo_root().join(format!("target/snapshot-diffs/{golden}.png")));
            panic!("{golden} changed: see target/snapshot-diffs/{golden}.png and target/{sheet_name}.png");
        }
    }
}

// ---------- sheets ----------

pub struct Sheet {
    w: usize,
    h: usize,
    px: Vec<Rgb>,
}

impl Sheet {
    pub fn new(w: usize, h: usize, bg: Rgb) -> Self {
        Self {
            w,
            h,
            px: vec![bg; w * h],
        }
    }

    pub fn tile(&mut self, t: &Tile, x: usize, y: usize, scale: usize) {
        for py in 0..t.side * scale {
            for px in 0..t.side * scale {
                let c = t.px[(py / scale) * t.side + px / scale];
                if x + px < self.w && y + py < self.h {
                    self.px[(y + py) * self.w + x + px] = c;
                }
            }
        }
    }

    pub fn text(&mut self, s: &str, x: usize, y: usize, scale: usize, c: Rgb) {
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
                                let (px, py) = (pen + col * scale + dx, y + row * scale + dy);
                                if px < self.w && py < self.h {
                                    self.px[py * self.w + px] = c;
                                }
                            }
                        }
                    }
                }
            }
            pen += (g.width as usize + 1) * scale;
        }
    }

    pub fn write(&self, path: &Path) {
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

pub fn read_png(path: &Path) -> Option<(usize, usize, Vec<u8>)> {
    let dec = png::Decoder::new(std::fs::File::open(path).ok()?);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width as usize, info.height as usize, buf))
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}
