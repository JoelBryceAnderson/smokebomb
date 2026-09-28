//! Firmware screens against the mockup's golden frames (SIM_SPEC H4).
//!
//! Runs the real firmware on the simulator HAL through the same timeline as a
//! capture scenario in `tools/mockup-capture`, and compares every face at
//! each checkpoint. Pixel-identical output isn't the goal (the mockup draws
//! on a 256 px canvas and downsamples; the firmware draws at 96×96), so each
//! face is scored by mean absolute difference in panel levels (0–15) after a
//! 3×3 blur, which forgives sub-pixel edge placement but not missing,
//! misplaced or mis-sized content.
//!
//! `SMOKEBOMB_GOLDEN_SHEETS=<dir>` writes a PNG per scenario: golden,
//! firmware and difference for every checkpoint.

use std::path::{Path, PathBuf};

use smokebomb_core::Firmware;
use smokebomb_hal::{Face, FACE_COUNT, FRAME_BYTES};
use smokebomb_hal_simulator::world::{TipDir, World, DEFAULT_VIEWER_RIGHT};
use smokebomb_hal_simulator::{imu_script, SimHandle, SimPlatform};

const FPS: f64 = 60.0;
const PIXELS: usize = 96 * 96;
type Levels = [u8; PIXELS];

/// Blurred MAE a face may have, in panel levels (0–15). Current worst cases
/// are ~0.16 on the 94 px result number and ~0.20 on text turned a quarter
/// (the success screen on a sideways face): the same glyphs in the same
/// place, anti-aliased differently by the browser's canvas, which rasterises
/// inside the rotated context where the firmware turns finished glyphs.
/// Content that is missing, misplaced by a pixel (≈ 1.0) or mis-sized scores
/// well above this.
const MAX_BLURRED_MAE: f32 = 0.25;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/mockup-capture/golden")
}

fn unpack(packed: &[u8]) -> Levels {
    let mut out = [0u8; PIXELS];
    for (i, px) in out.iter_mut().enumerate() {
        let b = packed[i / 2];
        *px = if i % 2 == 0 { b >> 4 } else { b & 0x0f };
    }
    out
}

fn load_golden(scenario: &str, name: &str) -> [Levels; FACE_COUNT] {
    let path = golden_dir().join(scenario).join(format!("{name}.bin"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(bytes.len(), FACE_COUNT * FRAME_BYTES);
    core::array::from_fn(|f| unpack(&bytes[f * FRAME_BYTES..(f + 1) * FRAME_BYTES]))
}

/// The firmware on the simulator, stepped frame by frame like the capture.
struct Run {
    sim: SimHandle,
    fw: Firmware<SimPlatform>,
    frame: u64,
    /// The simulator's die, for scenarios where it moves in the hand (the
    /// menu). Without it the IMU follows `imu_script`.
    world: Option<World>,
    /// A tip to start once the current frame is drawn: the mockup starts a
    /// tip from the frame before its first step, as it does the menu snap.
    pending_tip: Option<TipDir>,
}

impl Run {
    /// The mockup loads with +Y up.
    fn new() -> Self {
        Self::start(None)
    }

    /// Driven by the simulator's world model, like the simulator itself.
    fn with_world() -> Self {
        Self::start(Some(World::new()))
    }

    fn start(world: Option<World>) -> Self {
        let sim = SimHandle::new();
        {
            let mut s = sim.lock();
            s.manual_time_ms = Some(0);
            s.imu_resting = imu_script::resting(Face::PosY);
            // The mockup's stand-in battery level.
            s.battery_percent = 78;
        }
        let fw = Firmware::new(sim.peripherals()).unwrap();
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

    /// Run until the frame shown at mockup time `t` has been drawn.
    fn advance_to(&mut self, t: f64) {
        let target = (t * FPS).round() as u64;
        while self.frame <= target {
            self.step();
        }
    }

    /// A throw scripted to land with +Z up and roll 12 on the d20, like the
    /// mockup's `throw` scenario at seed 42.
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

    /// The mockup's `pressDie()`: a finger on the screen facing the viewer.
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
}

fn blur3(l: &Levels) -> [f32; PIXELS] {
    let mut out = [0f32; PIXELS];
    for y in 0..96i32 {
        for x in 0..96i32 {
            let mut sum = 0.0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (x + dx, y + dy);
                    if (0..96).contains(&xx) && (0..96).contains(&yy) {
                        sum += l[(yy * 96 + xx) as usize] as f32;
                    }
                }
            }
            out[(y * 96 + x) as usize] = sum / 9.0;
        }
    }
    out
}

struct Score {
    mae: f32,
    blurred: f32,
}

fn score(golden: &Levels, ours: &Levels) -> Score {
    let mae = golden
        .iter()
        .zip(ours)
        .map(|(&a, &b)| (a as f32 - b as f32).abs())
        .sum::<f32>()
        / PIXELS as f32;
    let (g, o) = (blur3(golden), blur3(ours));
    let blurred = g.iter().zip(&o).map(|(a, b)| (a - b).abs()).sum::<f32>() / PIXELS as f32;
    Score { mae, blurred }
}

struct Checkpoint {
    t: f64,
    name: &'static str,
    /// Faces left out of the comparison, and why.
    skip: &'static [(Face, &'static str)],
}

const DARK_FACE_DOWN: &[(Face, &str)] =
    &[(Face::NegY, "face-down stays dark (H2); the mockup lights it (G1)")];

struct Row {
    name: &'static str,
    golden: [Levels; FACE_COUNT],
    ours: [Levels; FACE_COUNT],
}

fn compare(scenario: &str, rows: &[Row], checkpoints: &[Checkpoint]) -> Vec<String> {
    let mut failures = Vec::new();
    println!("\n{scenario}: blurred MAE per face (+X −X +Y −Y +Z −Z), levels 0–15");
    for (row, cp) in rows.iter().zip(checkpoints) {
        let mut line = format!("  t={:>5}", row.name);
        for face in Face::ALL {
            if let Some((_, why)) = cp.skip.iter().find(|(f, _)| *f == face) {
                line += "     —";
                let _ = why;
                continue;
            }
            let s = score(&row.golden[face.index()], &row.ours[face.index()]);
            line += &format!(" {:5.2}", s.blurred);
            if s.blurred > MAX_BLURRED_MAE {
                failures.push(format!(
                    "{scenario} t={} face {face:?}: blurred MAE {:.2} (raw {:.2}) > {MAX_BLURRED_MAE}",
                    row.name, s.blurred, s.mae
                ));
            }
        }
        println!("{line}");
    }
    if let Some(dir) = std::env::var_os("SMOKEBOMB_GOLDEN_SHEETS") {
        write_sheet(Path::new(&dir), scenario, rows);
        for row in rows {
            let raw: Vec<u8> = row.ours.iter().flatten().copied().collect();
            std::fs::write(
                Path::new(&dir).join(format!("{scenario}-{}.levels", row.name)),
                raw,
            )
            .unwrap();
        }
    }
    failures
}

/// An input sent at a mockup time.
type Input = fn(&mut Run);

fn run_scenario(scenario: &str, checkpoints: &[Checkpoint], inputs: Vec<(f64, Input)>) -> Vec<String> {
    run_scenario_on(Run::new(), scenario, checkpoints, inputs)
}

fn run_scenario_on(
    mut run: Run,
    scenario: &str,
    checkpoints: &[Checkpoint],
    mut inputs: Vec<(f64, Input)>,
) -> Vec<String> {
    let mut rows = Vec::new();
    inputs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut inputs = inputs.into_iter().peekable();
    for cp in checkpoints {
        while let Some((t, _)) = inputs.peek() {
            if *t > cp.t {
                break;
            }
            let (t, input) = inputs.next().unwrap();
            run.advance_to(t - 1.0 / FPS);
            input(&mut run);
        }
        run.advance_to(cp.t);
        rows.push(Row {
            name: cp.name,
            golden: load_golden(scenario, cp.name),
            ours: run.faces(),
        });
    }
    compare(scenario, &rows, checkpoints)
}

#[test]
fn boot_and_wake_label_match_the_mockup() {
    // Smoke from the top-face burst (t ≈ 2.7–5.5 s) needs the particle
    // system, so those checkpoints wait for it.
    let times: &[(f64, &str)] = &[
        (0.1, "0.10"),
        (0.25, "0.25"),
        (0.5, "0.50"),
        (0.8, "0.80"),
        (1.2, "1.20"),
        (1.6, "1.60"),
        (2.0, "2.00"),
        (2.2, "2.20"),
        (2.35, "2.35"),
        (2.5, "2.50"),
        (2.7, "2.70"),
        (5.9, "5.90"),
        (6.1, "6.10"),
        (6.5, "6.50"),
        (7.5, "7.50"),
        (8.2, "8.20"),
        (8.6, "8.60"),
    ];
    let checkpoints: Vec<Checkpoint> = times
        .iter()
        .map(|&(t, name)| Checkpoint {
            t,
            name,
            skip: if t >= 6.2 { DARK_FACE_DOWN } else { &[] },
        })
        .collect();
    let mut failures = run_scenario("boot", &checkpoints, vec![]);

    let tap: Vec<Checkpoint> = [
        (9.1, "9.10"),
        (9.3, "9.30"),
        (9.6, "9.60"),
        (11.0, "11.00"),
        (11.8, "11.80"),
        (12.2, "12.20"),
    ]
    .into_iter()
    .map(|(t, name)| Checkpoint {
        t,
        name,
        skip: DARK_FACE_DOWN,
    })
    .collect();
    failures.extend(run_scenario("tap", &tap, vec![(9.0, Run::tap)]));

    assert!(
        failures.is_empty(),
        "faces too far from the mockup:\n{}",
        failures.join("\n")
    );
}

#[test]
fn result_screen_matches_the_mockup() {
    // The mockup rolled 12 on a d20 and landed +Z up, −Z down. Frames from
    // 15 s on are clear of smoke; 18.6–19.8 s cover the dim. The top face's
    // text angle depends on its orientation history (it keeps whatever it
    // last had as a side face), which a scripted throw doesn't reproduce.
    const TOP: &[(Face, &str)] = &[(Face::PosZ, "top face keeps its pre-throw angle")];
    let checkpoints: Vec<Checkpoint> = [
        (15.0, "15.00"),
        (17.0, "17.00"),
        (18.6, "18.60"),
        (19.2, "19.20"),
        (19.8, "19.80"),
        (20.5, "20.50"),
    ]
    .into_iter()
    .map(|(t, name)| Checkpoint { t, name, skip: TOP })
    .collect();
    // Starting the throw here makes the firmware reveal at 11.6 s, when the
    // mockup does (landing + 0.35 s).
    let failures = run_scenario("throw", &checkpoints, vec![(THROW_AT, Run::throw_12)]);
    assert!(
        failures.is_empty(),
        "faces too far from the mockup:\n{}",
        failures.join("\n")
    );
}

const THROW_AT: f64 = 10.4;

fn checkpoints(times: &[(f64, &'static str)], skip: &'static [(Face, &'static str)]) -> Vec<Checkpoint> {
    times
        .iter()
        .map(|&(t, name)| Checkpoint { t, name, skip })
        .collect()
}

#[test]
fn menu_matches_the_mockup() {
    // Hold +Z: the ring fills, the menu opens and the die turns to the
    // viewer. Tip left (page), up (value: d100), left, left, right, right,
    // then hold to save: success on the front face, the label elsewhere.
    let times: &[(f64, &str)] = &[
        (9.1, "9.10"),
        (9.3, "9.30"),
        (9.5, "9.50"),
        (9.7, "9.70"),
        (9.85, "9.85"),
        (10.0, "10.00"),
        (10.2, "10.20"),
        (10.7, "10.70"),
        (10.85, "10.85"),
        (11.1, "11.10"),
        (11.35, "11.35"),
        (11.7, "11.70"),
        (12.8, "12.80"),
        (13.9, "13.90"),
        (14.3, "14.30"),
        (14.6, "14.60"),
        (14.9, "14.90"),
        (15.0, "15.00"),
        (15.2, "15.20"),
        (15.5, "15.50"),
        (15.9, "15.90"),
        (16.3, "16.30"),
        (17.2, "17.20"),
    ];
    let inputs: Vec<(f64, Input)> = vec![
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
    ];
    // After the tip up, −X faces down: the mockup lights it for the save
    // flash and the setup label; the firmware keeps it dark (H2).
    const DOWN: &[(Face, &str)] = &[(Face::NegX, "face-down stays dark (H2); the mockup lights it (G1)")];
    let mut cps = checkpoints(times, &[]);
    for cp in cps.iter_mut().filter(|cp| cp.t > 14.8) {
        cp.skip = DOWN;
    }
    let failures = run_scenario_on(Run::with_world(), "menu", &cps, inputs);
    assert!(
        failures.is_empty(),
        "faces too far from the mockup:\n{}",
        failures.join("\n")
    );
}

#[test]
fn menu_settings_match_the_mockup() {
    // The mockup opens this one with its Menu button; a hold that ends when
    // the button is pressed gets the firmware to the same place.
    let mut times: Vec<(f64, &str)> = vec![(10.0, "10.00")];
    let names = [
        "item1", "item2", "item3", "item4", "item5", "item6", "item7", "item8", "item9",
    ];
    for (i, name) in names.iter().enumerate() {
        times.push((10.1 + 0.5 * i as f64 + 0.45, name));
    }
    let mut inputs: Vec<(f64, Input)> = vec![
        (8.2, Run::press),
        (9.3, Run::release),
        (9.5, |r| r.tip(TipDir::Right)),
    ];
    for i in 0..9 {
        inputs.push((10.1 + 0.5 * i as f64, |r| r.tip(TipDir::Up)));
    }
    let failures = run_scenario_on(
        Run::with_world(),
        "menuSettings",
        &checkpoints(&times, &[]),
        inputs,
    );
    assert!(
        failures.is_empty(),
        "faces too far from the mockup:\n{}",
        failures.join("\n")
    );
}

#[test]
fn scripted_throw_reveals_when_the_mockup_does() {
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

fn write_sheet(dir: &Path, scenario: &str, rows: &[Row]) {
    const S: usize = 2;
    let (w, gap, label) = (96 * S, 4, 0);
    let width = label + FACE_COUNT * (w + gap);
    let height = rows.len() * 3 * (w + gap);
    let mut img = vec![20u8; width * height];
    let mut blit = |x0: usize, y0: usize, px: &dyn Fn(usize) -> u8| {
        for y in 0..w {
            for x in 0..w {
                img[(y0 + y) * width + x0 + x] = px((y / S) * 96 + x / S);
            }
        }
    };
    for (r, row) in rows.iter().enumerate() {
        for f in 0..FACE_COUNT {
            let x0 = label + f * (w + gap);
            let y0 = r * 3 * (w + gap);
            blit(x0, y0, &|i| row.golden[f][i] * 17);
            blit(x0, y0 + w + gap, &|i| row.ours[f][i] * 17);
            blit(x0, y0 + 2 * (w + gap), &|i| {
                ((row.golden[f][i] as i16 - row.ours[f][i] as i16).unsigned_abs() as u8 * 17)
                    .saturating_mul(2)
            });
        }
    }
    std::fs::create_dir_all(dir).unwrap();
    let file = std::fs::File::create(dir.join(format!("{scenario}.png"))).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
    enc.set_color(png::ColorType::Grayscale);
    enc.write_header().unwrap().write_image_data(&img).unwrap();
}
