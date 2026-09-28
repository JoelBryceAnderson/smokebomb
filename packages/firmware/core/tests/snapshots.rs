//! Screen snapshots: what the six faces show at chosen moments.
//!
//! Each scenario drives the real firmware on the simulator HAL through a
//! scripted timeline (seeded randomness, virtual time) and compares every
//! face at every checkpoint with the reference in `tests/snapshots/`. A
//! reference is one PNG contact sheet: a row per checkpoint, the six faces
//! left to right (+X −X +Y −Y +Z −Z), 4-bit grayscale so a pixel's value is
//! its panel level (0–15). GitHub shows a changed sheet as an image diff.
//!
//! When a change to the screens is intended, regenerate the sheets and review
//! them like any other diff:
//!
//! ```sh
//! UPDATE_SNAPSHOTS=1 cargo test -p smokebomb-core --test snapshots
//! ```
//!
//! A failing run writes expected, actual and difference for every checkpoint
//! to `target/snapshot-diffs/<scenario>.png` (or `SMOKEBOMB_SNAPSHOT_DIR`).

use std::path::{Path, PathBuf};

use smokebomb_core::smoke::SmokeRng;
use smokebomb_core::Firmware;
use smokebomb_hal::{Face, FACE_COUNT};
use smokebomb_hal_simulator::world::{TipDir, World, DEFAULT_VIEWER_RIGHT};
use smokebomb_hal_simulator::{imu_script, SimHandle, SimPlatform};

const FPS: f64 = 60.0;
const SIDE: usize = 96;
const PIXELS: usize = SIDE * SIDE;
type Levels = [u8; PIXELS];

/// The most a face's mean absolute difference from its reference may be, in
/// panel levels (0–15). The frames are deterministic, so this only absorbs
/// floating-point differences between platforms: a changed glyph, a shifted
/// edge or a different smoke particle is far above it.
const MAX_FACE_MAE: f32 = 0.05;

/// The world model's seed, so shakes and tumbles are the same every run.
const WORLD_SEED: u64 = 1;
/// Seeds the smoke.
const SMOKE_SEED: u32 = 42;

fn unpack(packed: &[u8]) -> Levels {
    let mut out = [0u8; PIXELS];
    for (i, px) in out.iter_mut().enumerate() {
        let b = packed[i / 2];
        *px = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
    }
    out
}

/// The firmware on the simulator, stepped frame by frame.
struct Run {
    sim: SimHandle,
    fw: Firmware<SimPlatform>,
    frame: u64,
    /// The simulator's die, for scenarios where it moves in the hand (the
    /// menu). Without it the IMU follows `imu_script`.
    world: Option<World>,
    /// A tip to start once the current frame is drawn.
    pending_tip: Option<TipDir>,
}

impl Run {
    /// The die starts with +Y up.
    fn new() -> Self {
        Self::start(None)
    }

    /// Driven by the simulator's world model, like the simulator itself.
    fn with_world() -> Self {
        Self::start(Some(World::with_seed(WORLD_SEED)))
    }

    fn start(world: Option<World>) -> Self {
        let sim = SimHandle::new();
        {
            let mut s = sim.lock();
            s.manual_time_ms = Some(0);
            s.imu_resting = imu_script::resting(Face::PosY);
            s.battery_percent = 78;
        }
        let mut fw = Firmware::new(sim.peripherals()).unwrap();
        fw.smoke_mut().set_rng(SmokeRng::new(SMOKE_SEED));
        let mut run = Self {
            sim,
            fw,
            frame: 0,
            world,
            pending_tip: None,
        };
        run.step();
        run
    }

    fn step(&mut self) {
        let menu_before = self.fw.menu_front();
        {
            let mut s = self.sim.lock();
            s.manual_time_ms = Some((self.frame as f64 * 1000.0 / FPS).round() as u64);
            if let Some(w) = &mut self.world {
                // World time matches frame time: frame 0 is at t = 0.
                s.imu_resting = w.step(if self.frame == 0 { 0.0 } else { 1.0 / FPS });
            }
        }
        self.fw.tick().unwrap();
        // As the simulator server does: turn the held face to the viewer.
        if let (None, Some(face), Some(w)) = (menu_before, self.fw.menu_front(), &mut self.world) {
            w.snap_to_viewer(face);
        }
        if let (Some(dir), Some(w)) = (self.pending_tip.take(), &mut self.world) {
            assert!(w.tip(dir, DEFAULT_VIEWER_RIGHT), "die busy");
        }
        self.frame += 1;
    }

    /// Run until the frame shown at time `t` has been drawn.
    fn advance_to(&mut self, t: f64) {
        let target = (t * FPS).round() as u64;
        while self.frame <= target {
            self.step();
        }
    }

    /// A throw scripted to land with +Z up and roll 12 on the d20.
    fn throw_12(&mut self) {
        let mut s = self.sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosZ);
        // uniform(20) maps x to x % 20 + 1.
        s.rng_script.push_back(11);
    }

    /// A quick touch on a screen.
    fn tap(&mut self) {
        self.sim.lock().touch_mask = 1 << Face::PosZ.index();
        self.step();
        self.sim.lock().touch_mask = 0;
    }

    /// A finger on the screen facing the viewer.
    fn press(&mut self) {
        self.sim.lock().touch_mask = 1 << Face::PosZ.index();
    }

    fn release(&mut self) {
        self.sim.lock().touch_mask = 0;
    }

    fn tip(&mut self, dir: TipDir) {
        assert!(self.world.is_some(), "tips need the world model");
        self.pending_tip = Some(dir);
    }

    fn faces(&self) -> [Levels; FACE_COUNT] {
        let s = self.sim.lock();
        core::array::from_fn(|f| unpack(&s.faces[f]))
    }

    fn shake(&mut self) {
        self.world.as_mut().expect("the world model").start_shake();
    }

    fn throw_release(&mut self) {
        self.world.as_mut().expect("the world model").end_shake(true);
    }
}

// ---------- sheets ----------

/// Space between faces in a sheet, and its level (a dark gray, so the
/// boundaries show).
const GAP: usize = 4;
const GAP_LEVEL: u8 = 3;

type Row = [Levels; FACE_COUNT];

fn snapshot_path(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{scenario}.png"))
}

fn diff_dir() -> PathBuf {
    std::env::var_os("SMOKEBOMB_SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/snapshot-diffs"))
}

fn sheet_size(rows: usize) -> (usize, usize) {
    (
        FACE_COUNT * SIDE + (FACE_COUNT - 1) * GAP,
        rows * SIDE + rows.saturating_sub(1) * GAP,
    )
}

/// The sheet's levels, one byte per pixel.
fn render_sheet(rows: &[Row]) -> Vec<u8> {
    let (w, h) = sheet_size(rows.len());
    let mut img = vec![GAP_LEVEL; w * h];
    for (r, row) in rows.iter().enumerate() {
        for (f, face) in row.iter().enumerate() {
            let (x0, y0) = (f * (SIDE + GAP), r * (SIDE + GAP));
            for y in 0..SIDE {
                let at = (y0 + y) * w + x0;
                img[at..at + SIDE].copy_from_slice(&face[y * SIDE..(y + 1) * SIDE]);
            }
        }
    }
    img
}

fn write_sheet(path: &Path, rows: &[Row]) {
    let (w, h) = sheet_size(rows.len());
    let img = render_sheet(rows);
    // 4-bit samples, high nibble first, each row padded to a byte.
    let stride = w.div_ceil(2);
    let mut packed = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            packed[y * stride + x / 2] |= img[y * w + x] << if x % 2 == 0 { 4 } else { 0 };
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Four);
    enc.write_header().unwrap().write_image_data(&packed).unwrap();
}

fn read_sheet(path: &Path) -> Option<Vec<Row>> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file))
        .read_info()
        .ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    assert_eq!(
        (info.color_type, info.bit_depth),
        (png::ColorType::Grayscale, png::BitDepth::Four),
        "{}: not a snapshot sheet",
        path.display()
    );
    let rows = (h + GAP) / (SIDE + GAP);
    assert_eq!(
        (w, h),
        sheet_size(rows),
        "{}: not a snapshot sheet",
        path.display()
    );
    let stride = w.div_ceil(2);
    let level = |x: usize, y: usize| {
        let b = buf[y * stride + x / 2];
        if x % 2 == 0 {
            b >> 4
        } else {
            b & 0x0f
        }
    };
    Some(
        (0..rows)
            .map(|r| {
                core::array::from_fn(|f| {
                    let mut face = [0u8; PIXELS];
                    for (i, px) in face.iter_mut().enumerate() {
                        *px = level(f * (SIDE + GAP) + i % SIDE, r * (SIDE + GAP) + i / SIDE);
                    }
                    face
                })
            })
            .collect(),
    )
}

/// Expected, actual and their difference (×2), for looking at a failure.
fn write_diff_sheet(path: &Path, expected: &[Row], actual: &[Row]) {
    const S: usize = 2;
    let (w, gap) = (SIDE * S, 4);
    let rows = actual.len();
    let width = FACE_COUNT * (w + gap);
    let height = rows * 3 * (w + gap);
    let mut img = vec![20u8; width * height];
    let mut blit = |x0: usize, y0: usize, px: &dyn Fn(usize) -> u8| {
        for y in 0..w {
            for x in 0..w {
                img[(y0 + y) * width + x0 + x] = px((y / S) * SIDE + x / S);
            }
        }
    };
    for (r, ours) in actual.iter().enumerate() {
        for f in 0..FACE_COUNT {
            let (x0, y0) = (f * (w + gap), r * 3 * (w + gap));
            match expected.get(r) {
                Some(exp) => {
                    blit(x0, y0, &|i| exp[f][i] * 17);
                    blit(x0, y0 + 2 * (w + gap), &|i| {
                        ((exp[f][i] as i16 - ours[f][i] as i16).unsigned_abs() as u8 * 17).saturating_mul(2)
                    });
                }
                None => blit(x0, y0, &|_| 0),
            }
            blit(x0, y0 + w + gap, &|i| ours[f][i] * 17);
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.write_header().unwrap().write_image_data(&img).unwrap();
}

fn mae(a: &Levels, b: &Levels) -> f32 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| (x as f32 - y as f32).abs())
        .sum::<f32>()
        / PIXELS as f32
}

/// Compare the scenario's rows with its sheet, or write the sheet when
/// `UPDATE_SNAPSHOTS` is set. Returns what differs.
fn check(scenario: &str, names: &[&str], rows: &[Row]) -> Vec<String> {
    let path = snapshot_path(scenario);
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        write_sheet(&path, rows);
        println!("{scenario}: wrote {}", path.display());
        return vec![];
    }
    let Some(expected) = read_sheet(&path) else {
        write_diff_sheet(&diff_dir().join(format!("{scenario}.png")), &[], rows);
        return vec![format!(
            "{scenario}: no snapshot at {}; run with UPDATE_SNAPSHOTS=1 to create it",
            path.display()
        )];
    };
    let mut failures = Vec::new();
    if expected.len() != rows.len() {
        failures.push(format!(
            "{scenario}: the snapshot has {} checkpoints, the scenario {}",
            expected.len(),
            rows.len()
        ));
    }
    println!("\n{scenario}: mean absolute difference per face (+X −X +Y −Y +Z −Z), levels 0–15");
    for ((name, row), exp) in names.iter().zip(rows).zip(&expected) {
        let mut line = format!("  t={name:>6}");
        for face in Face::ALL {
            let d = mae(&exp[face.index()], &row[face.index()]);
            line += &format!(" {d:6.3}");
            if d > MAX_FACE_MAE {
                failures.push(format!(
                    "{scenario} t={name} face {face:?}: differs from the snapshot by {d:.3} levels (limit {MAX_FACE_MAE})"
                ));
            }
        }
        println!("{line}");
    }
    if !failures.is_empty() {
        write_diff_sheet(&diff_dir().join(format!("{scenario}.png")), &expected, rows);
    }
    failures
}

// ---------- scenarios ----------

/// An input sent at a time.
type Input = fn(&mut Run);

/// Run `run` through the timeline, capturing every face at each checkpoint
/// `(time, name)` and checking them against the scenario's snapshot.
fn run_scenario(scenario: &str, mut run: Run, times: &[(f64, &str)], mut inputs: Vec<(f64, Input)>) {
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut inputs = inputs.into_iter().peekable();
    let mut rows = Vec::new();
    for &(t, _) in times {
        while let Some((at, _)) = inputs.peek() {
            if *at > t {
                break;
            }
            let (at, input) = inputs.next().unwrap();
            run.advance_to(at - 1.0 / FPS);
            input(&mut run);
        }
        run.advance_to(t);
        rows.push(run.faces());
    }
    let names: Vec<&str> = times.iter().map(|&(_, name)| name).collect();
    let failures = check(scenario, &names, &rows);
    assert!(failures.is_empty(), "screens changed:\n{}", failures.join("\n"));
}

/// Checkpoints named by their time.
fn at(times: &[f64]) -> Vec<(f64, String)> {
    times.iter().map(|&t| (t, format!("{t:.2}"))).collect()
}

fn run_timeline(scenario: &str, run: Run, times: &[f64], inputs: Vec<(f64, Input)>) {
    let named = at(times);
    let times: Vec<(f64, &str)> = named.iter().map(|(t, n)| (*t, n.as_str())).collect();
    run_scenario(scenario, run, &times, inputs);
}

#[test]
fn boot() {
    // 2.7–5.5 s is the top face's smoke burst.
    run_timeline(
        "boot",
        Run::new(),
        &[
            0.1, 0.25, 0.5, 0.8, 1.2, 1.6, 2.0, 2.2, 2.35, 2.5, 2.7, 3.0, 3.6, 4.2, 5.0, 5.9, 6.1, 6.5, 7.5,
            8.2, 8.6,
        ],
        vec![],
    );
}

#[test]
fn tap_shows_the_setup() {
    run_timeline(
        "tap",
        Run::new(),
        &[9.1, 9.3, 9.6, 11.0, 11.8, 12.2],
        vec![(9.0, Run::tap)],
    );
}

const THROW_AT: f64 = 10.4;

#[test]
fn result_screen() {
    // A scripted throw rolls 12 on a d20 and lands +Z up, −Z down. The
    // result shows at 11.6 s; frames from 15 s on are clear of smoke, and
    // 18.6–19.8 s cover the dim.
    run_timeline(
        "result",
        Run::new(),
        &[15.0, 17.0, 18.6, 19.2, 19.8, 20.5],
        vec![(THROW_AT, Run::throw_12)],
    );
}

#[test]
fn quick_throw_smoke() {
    // A quick press throws with almost no shake: the throw fills the cloud.
    let run = Run::with_world();
    run.sim.lock().rng_script.push_back(11);
    run_timeline(
        "quick-throw-smoke",
        run,
        &[9.3, 10.6, 11.2],
        vec![(9.0, Run::shake), (9.05, Run::throw_release)],
    );
}

#[test]
fn throw_smoke() {
    // Hold the throw button 1 s, release: the shake fills the cloud, the
    // tumble carries it, the landing drains it with embers. It rolls 12 and
    // lands +Z up.
    let mut run = Run::with_world();
    run.world.as_mut().unwrap().set_next_landing(Face::PosZ);
    run.sim.lock().rng_script.push_back(11);
    run_timeline(
        "throw-smoke",
        run,
        &[
            9.2, 9.5, 9.9, 10.2, 10.6, 11.0, 11.3, 11.45, 11.6, 11.8, 12.1, 12.5, 13.5,
        ],
        vec![(9.0, Run::shake), (10.0, Run::throw_release)],
    );
}

#[test]
fn menu() {
    // Hold +Z: the ring fills, the menu opens and the die turns to the
    // viewer. Tip left (page), up (value: d100), left, left, right, right,
    // then hold to save: success on the front face, the label elsewhere.
    run_timeline(
        "menu",
        Run::with_world(),
        &[
            9.1, 9.3, 9.5, 9.7, 9.85, 10.0, 10.2, 10.7, 10.85, 11.1, 11.35, 11.7, 12.8, 13.9, 14.3, 14.6,
            14.9, 15.0, 15.2, 15.5, 15.9, 16.3, 17.2,
        ],
        vec![
            (9.0, Run::press),
            (10.3, Run::release),
            (10.6, |r| r.tip(TipDir::Left)),
            (11.2, |r| r.tip(TipDir::Up)),
            (11.8, |r| r.tip(TipDir::Left)),
            (12.3, |r| r.tip(TipDir::Left)),
            (12.9, |r| r.tip(TipDir::Right)),
            (13.4, |r| r.tip(TipDir::Right)),
            (14.0, Run::press),
            (14.85, Run::release),
        ],
    );
}

#[test]
fn menu_settings() {
    // Open the menu, tip to the Settings page, then tip up through the items.
    const ITEMS: usize = 9;
    let names: Vec<String> = (1..=ITEMS).map(|i| format!("item{i}")).collect();
    let mut times: Vec<(f64, &str)> = vec![(10.0, "10.00")];
    for (i, name) in names.iter().enumerate() {
        times.push((10.1 + 0.5 * i as f64 + 0.45, name));
    }
    let mut inputs: Vec<(f64, Input)> = vec![
        // Open by 8.97 s, so the die has turned to the viewer and held still
        // (0.5 s in all) before the first tip at 9.5 s.
        (8.1, Run::press),
        (9.3, Run::release),
        (9.5, |r| r.tip(TipDir::Right)),
    ];
    for i in 0..ITEMS {
        inputs.push((10.1 + 0.5 * i as f64, |r| r.tip(TipDir::Up)));
    }
    run_scenario("menu-settings", Run::with_world(), &times, inputs);
}

#[test]
fn a_scripted_throw_reveals_after_landing() {
    let mut run = Run::new();
    run.advance_to(THROW_AT - 1.0 / FPS);
    run.throw_12();
    while !matches!(run.fw.mode(), smokebomb_core::state::Mode::Reveal { .. }) {
        run.step();
        assert!(run.frame < 20 * 60, "never revealed");
    }
    let t = run.frame as f64 / FPS;
    assert!((t - 11.6).abs() < 0.05, "revealed at {t:.3} s");
    assert_eq!(run.fw.last_roll().unwrap().record.values.as_slice(), &[12]);
}
