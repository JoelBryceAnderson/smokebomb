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

use smokebomb_core::menu::Settings;
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
/// floating-point differences between platforms.
const MAX_FACE_MAE: f32 = 0.05;
/// The most pixels of a face that may differ from the reference by two levels
/// or more. Rounding noise moves a few pixels by one level; a changed digit
/// in small text moves dozens by much more, yet barely changes the mean.
const MAX_BIG_DIFFS: usize = 4;

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
        Self::start(None, false)
    }

    /// Driven by the simulator's world model, like the simulator itself.
    fn with_world() -> Self {
        Self::start(Some(World::with_seed(WORLD_SEED)), false)
    }

    /// The world model with the default menu, which has a Mode page.
    fn with_world_and_modes() -> Self {
        Self::start(Some(World::with_seed(WORLD_SEED)), true)
    }

    fn start(world: Option<World>, modes: bool) -> Self {
        let sim = SimHandle::new();
        {
            let mut s = sim.lock();
            s.manual_time_ms = Some(0);
            s.imu_resting = imu_script::resting(Face::PosY);
            s.battery_percent = 78;
        }
        let mut fw = Firmware::new(sim.peripherals()).unwrap();
        // Most menu scenarios were written for the plain three-page dice
        // menu; `menu-modes` covers the default one, with its Mode page.
        fw.set_settings(Settings {
            modes,
            ..Settings::default()
        });
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
                let dt = if self.frame == 0 { 0.0 } else { 1.0 / FPS };
                s.imu_resting = w.step(dt);
                s.sync_world(w, dt);
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

/// How many pixels differ by two levels or more.
fn big_diffs(a: &Levels, b: &Levels) -> usize {
    a.iter().zip(b).filter(|(&x, &y)| x.abs_diff(y) >= 2).count()
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
    println!("\n{scenario}: mean difference / pixels off by 2+ per face (+X −X +Y −Y +Z −Z)");
    for ((name, row), exp) in names.iter().zip(rows).zip(&expected) {
        let mut line = format!("  t={name:>6}");
        for face in Face::ALL {
            let (a, b) = (&exp[face.index()], &row[face.index()]);
            let (d, big) = (mae(a, b), big_diffs(a, b));
            line += &format!(" {d:6.3}/{big:<4}");
            if d > MAX_FACE_MAE || big > MAX_BIG_DIFFS {
                failures.push(format!(
                    "{scenario} t={name} face {face:?}: differs from the snapshot by {d:.3} levels on average, with {big} pixels off by 2 or more (limits {MAX_FACE_MAE}, {MAX_BIG_DIFFS})"
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

/// The setup label's icon for every die, Pass the Pot and Hot Potato: pick
/// the setup, tap, and capture the label at full brightness.
#[test]
fn setup_icons() {
    use smokebomb_core::menu::PlayMode;
    use smokebomb_shared::DieKind;

    fn pick(run: &mut Run, play: PlayMode, die: DieKind) {
        run.fw.set_settings(Settings {
            modes: true,
            play,
            die,
            ..Settings::default()
        });
        run.tap();
    }
    macro_rules! die {
        ($k:ident) => {
            (|run: &mut Run| pick(run, PlayMode::Dice, DieKind::$k)) as Input
        };
    }
    let inputs: Vec<(f64, Input)> = vec![
        (9.0, die!(D4)),
        (10.0, die!(D6)),
        (11.0, die!(D8)),
        (12.0, die!(D10)),
        (13.0, die!(D12)),
        (14.0, die!(D20)),
        (15.0, die!(D100)),
        (16.0, |run| pick(run, PlayMode::PassThePot, DieKind::D6)),
        (17.0, |run| pick(run, PlayMode::HotPotato, DieKind::D6)),
    ];
    let times: Vec<(f64, &str)> = vec![
        (9.5, "d4"),
        (10.5, "d6"),
        (11.5, "d8"),
        (12.5, "d10"),
        (13.5, "d12"),
        (14.5, "d20"),
        (15.5, "d100"),
        (16.5, "pot"),
        (17.5, "potato"),
    ];
    run_scenario("setup-icons", Run::new(), &times, inputs);
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

/// A random word that lands the pigs on `pose`.
fn pose_word(pose: smokebomb_core::pigs::Pose) -> u32 {
    use smokebomb_core::pigs::{Pose, WEIGHT_TOTAL};
    let start: u32 = Pose::ALL[..pose.index()].iter().map(|p| p.weight() as u32).sum();
    let mid = start * 2 + pose.weight() as u32; // twice the bucket's midpoint
    ((mid as u64 * (u32::MAX as u64 + 1)) / (2 * WEIGHT_TOTAL as u64)) as u32
}

impl Run {
    /// A throw scripted to land +Z up with the two pigs in these poses.
    fn throw_pigs(&mut self, poses: [smokebomb_core::pigs::Pose; 2]) {
        let mut s = self.sim.lock();
        s.imu_script.extend(imu_script::throw());
        s.imu_resting = imu_script::resting(Face::PosZ);
        for p in poses {
            s.rng_script.push_back(pose_word(p));
        }
    }
}

fn pigs_scenario(name: &str, throw: Input) {
    let run = Run::new().with_settings(|s| {
        s.modes = true;
        s.play = smokebomb_core::menu::PlayMode::PigToss;
    });
    run_timeline(
        name,
        run,
        &[10.6, 11.0, 11.4, 11.8, 12.2, 12.7, 15.0],
        vec![(THROW_AT, throw)],
    );
}

#[test]
fn pigs_score() {
    use smokebomb_core::pigs::Pose::*;
    pigs_scenario("pigs-score", |r| r.throw_pigs([Back, Feet]));
}

#[test]
fn pigs_bust() {
    use smokebomb_core::pigs::Pose::*;
    pigs_scenario("pigs-bust", |r| r.throw_pigs([SideDot, SidePlain]));
}

#[test]
fn pigs_rare() {
    use smokebomb_core::pigs::Pose::*;
    pigs_scenario("pigs-rare", |r| r.throw_pigs([Nose, Ear]));
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

impl Run {
    /// Change some settings before the scenario starts.
    fn with_settings(mut self, edit: impl FnOnce(&mut Settings)) -> Self {
        let mut settings = *self.fw.settings();
        edit(&mut settings);
        self.fw.set_settings(settings);
        self
    }
}

/// Hold the throw button 1 s, release: the shake fills the cloud, the tumble
/// carries it, the landing drains it with embers. It rolls 12 and lands +Z up.
fn throw_smoke_scenario(scenario: &str, mut run: Run) {
    run.world.as_mut().unwrap().set_next_landing(Face::PosZ);
    run.sim.lock().rng_script.push_back(11);
    run_timeline(
        scenario,
        run,
        &[
            9.2, 9.5, 9.9, 10.2, 10.6, 11.0, 11.3, 11.45, 11.6, 11.8, 12.1, 12.5, 13.5,
        ],
        vec![(9.0, Run::shake), (10.0, Run::throw_release)],
    );
}

#[test]
fn throw_smoke() {
    throw_smoke_scenario("throw-smoke", Run::with_world());
}

/// The Smoke setting is the third Settings item: Off, Light, Full.
const SMOKE_ITEM: usize = 2;

#[test]
fn throw_smoke_light() {
    let run = Run::with_world().with_settings(|s| s.choices[SMOKE_ITEM] = 1);
    throw_smoke_scenario("throw-smoke-light", run);
}

#[test]
fn throw_smoke_off() {
    let run = Run::with_world().with_settings(|s| s.choices[SMOKE_ITEM] = 0);
    throw_smoke_scenario("throw-smoke-off", run);
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
    const ITEMS: usize = 7;
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
fn menu_modes() {
    // The default menu opens on How many dice; one tip right is the Mode
    // page. Choose Pass the Pot, then Hot Potato, tip to its Fuse length and
    // pick Long, then hold to save: the success screen says how to start.
    run_timeline(
        "menu-modes",
        Run::with_world_and_modes(),
        &[10.0, 10.55, 11.05, 11.55, 12.05, 13.4, 13.9, 15.5],
        vec![
            (8.1, Run::press),
            (9.3, Run::release),
            (9.5, |r| r.tip(TipDir::Right)),
            (10.1, |r| r.tip(TipDir::Up)),
            (10.6, |r| r.tip(TipDir::Up)),
            (11.1, |r| r.tip(TipDir::Left)),
            (11.6, |r| r.tip(TipDir::Up)),
            (12.3, Run::press),
            (13.25, Run::release),
        ],
    );
}

#[test]
fn menu_bills_in_hand() {
    // Pass the Pot: the page counts the bills in your hand, which is how many
    // dice you roll. It starts at three; tip down to two and save.
    run_timeline(
        "menu-bills",
        Run::with_world_and_modes(),
        &[10.0, 10.55, 11.05, 11.55, 13.4, 15.5],
        vec![
            (8.1, Run::press),
            (9.3, Run::release),
            (9.5, |r| r.tip(TipDir::Right)),
            (10.1, |r| r.tip(TipDir::Up)),
            (10.6, |r| r.tip(TipDir::Left)),
            (11.1, |r| r.tip(TipDir::Down)),
            (11.7, Run::press),
            (12.65, Run::release),
        ],
    );
}

#[test]
fn menu_settings_tap_and_power_off() {
    // On the Settings page a tap changes the item (Brightness 70% → 100%).
    // Tip down past Regulatory and About to Power off and tap: every screen
    // goes dark and stays dark until a tap boots the die.
    run_timeline(
        "menu-settings-tap",
        Run::with_world(),
        &[10.0, 10.6, 11.1, 12.3, 13.0, 20.0, 20.6, 27.5],
        vec![
            (8.1, Run::press),
            (9.3, Run::release),
            (9.5, |r| r.tip(TipDir::Right)),
            (10.2, Run::tap),
            (10.7, |r| r.tip(TipDir::Down)),
            (11.2, |r| r.tip(TipDir::Down)),
            (11.7, |r| r.tip(TipDir::Down)),
            (12.6, Run::tap),
            (20.2, Run::tap),
        ],
    );
}

// ---------- the Nest ----------

fn world_of(run: &mut Run) -> &mut World {
    run.world.as_mut().expect("the world model")
}

fn dock_right(run: &mut Run) {
    world_of(run).place_in_nest(Face::NegY, 0);
}

fn dock_charging_face_up(run: &mut Run) {
    world_of(run).place_in_nest(Face::PosY, 0);
}

fn dock_charging_face_beside(run: &mut Run) {
    world_of(run).place_in_nest(Face::PosX, 0);
}

fn dock_unplugged(run: &mut Run) {
    run.sim.lock().nest_plugged = false;
    world_of(run).place_in_nest(Face::NegY, 0);
}

fn lift(run: &mut Run) {
    world_of(run).lift();
}

fn fault_on(run: &mut Run) {
    run.sim.lock().charger_fault = true;
}

#[test]
fn nest_dock_animation() {
    // Placed at 7 s; it stills at about 8 s, seats by 8.5 s, and the dock
    // animation runs 8.5–10.1 s; the display holds after that.
    run_timeline(
        "nest-dock",
        Run::with_world(),
        &[7.4, 8.6, 8.8, 9.0, 9.2, 9.4, 9.6, 9.8, 10.2, 11.0, 12.5],
        vec![(7.0, dock_right)],
    );
}

#[test]
fn nest_undock() {
    run_timeline(
        "nest-undock",
        Run::with_world(),
        &[12.0, 13.0, 13.2, 13.4, 13.6, 14.0, 14.4, 15.0, 16.0],
        vec![(7.0, dock_right), (13.0, lift)],
    );
}

#[test]
fn nest_charging_face_on_top() {
    run_timeline(
        "nest-flip",
        Run::with_world(),
        &[9.5, 10.0, 10.3, 10.6, 10.9],
        vec![(7.0, dock_charging_face_up)],
    );
}

#[test]
fn nest_charging_face_beside() {
    run_timeline(
        "nest-tip",
        Run::with_world(),
        &[9.5, 9.9, 10.3],
        vec![(7.0, dock_charging_face_beside)],
    );
}

#[test]
fn nest_no_power_and_fault() {
    run_timeline(
        "nest-no-power",
        Run::with_world(),
        &[12.0, 13.0],
        vec![(7.0, dock_unplugged)],
    );
    run_timeline(
        "nest-fault",
        Run::with_world(),
        &[12.0, 13.0],
        vec![(7.0, dock_right), (10.0, fault_on)],
    );
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

#[test]
fn hot_potato_round() {
    // Shake to light the fuse (a 15 s fuse from the TRNG), watch it heat up,
    // and go off.
    let run =
        Run::with_world_and_modes().with_settings(|s| s.play = smokebomb_core::menu::PlayMode::HotPotato);
    run.sim.lock().rng_script.push_back(1 << 31);
    run_timeline(
        "hot-potato",
        run,
        &[10.5, 14.0, 18.0, 22.0, 25.0, 27.0, 29.0],
        vec![(9.0, Run::shake), (9.4, Run::throw_release)],
    );
}
