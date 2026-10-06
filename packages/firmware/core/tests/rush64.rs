//! Sugar Rush, from the whole firmware on the simulator: tip the die,
//! tap the top screen, and the stick slides off. Played on the 30 mm die's
//! 64×64 panels and the 34 mm die's 96×96 with the same inputs.
//!
//! * `tests/snapshots/rgb64-rush.png`: every 64×64 checkpoint at 1:1,
//!   compared exactly (`UPDATE_SNAPSHOTS=1` regenerates it).
//! * `docs/30mm/contact-sheet-rush.png` (with `UPDATE_SNAPSHOTS=1`, and
//!   always in `target/`): the 64×64 top face at 4× over the 96×96 at 3×.

mod common;

use common::{Run, Tile};
use smokebomb_core::menu::{Grid, PlayMode, Settings};
use smokebomb_core::rush::{self, Phase};
use smokebomb_core::state::Mode;
use smokebomb_core::target::DisplayTarget;
use smokebomb_hal::{Face, Grey96, HapticEffect, ImuSample, Rgb64};
use smokebomb_shared::ModeSet;

/// The board every run plays.
const SEED: u32 = 0x5EED_0001;

fn settings(grid: Grid) -> Settings {
    Settings {
        enabled: ModeSet::ALL,
        play: PlayMode::SugarRush,
        grid,
        ..Settings::default()
    }
}

/// A Sugar Rush game on the Small board from [`SEED`], past the boot.
fn start<T: DisplayTarget>() -> Run<T> {
    let mut run = Run::<T>::new(settings(Grid::Big));
    run.sim.lock().rng_script.push_back(SEED);
    run.fw.set_settings(settings(Grid::Small));
    run.advance_to(6.5);
    run
}

/// Resting with `top` up, leaning `deg` degrees so that `downhill` (a
/// direction along the top face) is lower.
fn lean(top: Face, downhill: Option<rush::V>, deg: f32) -> ImuSample {
    let (_, _, n) = rush::basis(top);
    let (s, c) = deg.to_radians().sin_cos();
    let d = downhill.unwrap_or([0; 3]);
    // The accelerometer reads +1 g along up; leaning lowers `downhill`, so
    // up tips away from it.
    let up = [0, 1, 2].map(|i| n[i] as f32 * c - d[i] as f32 * s);
    ImuSample {
        accel_mg: up.map(|u| (u * 1000.0).round() as i16),
        gyro_mdps: [0; 3],
    }
}

fn hold_die<T: DisplayTarget>(run: &mut Run<T>, sample: ImuSample) {
    run.sim.lock().imu_resting = sample;
}

/// A short touch on `face`, with `held` faces touched throughout (a grip).
fn tap_face<T: DisplayTarget>(run: &mut Run<T>, face: Face, held: u8) {
    run.sim.lock().touch_mask = held | 1 << face.index();
    for _ in 0..6 {
        run.step();
    }
    run.sim.lock().touch_mask = held;
    run.step();
}

fn advance<T: DisplayTarget>(run: &mut Run<T>, secs: f64) {
    for _ in 0..(secs * common::FPS).round() as usize {
        run.step();
    }
}

/// A stick that can go, preferring one whose head is on `prefer`.
fn free_stick<T: DisplayTarget>(run: &Run<T>, prefer: Face) -> usize {
    let r = run.fw.rush();
    let free: Vec<usize> = (0..r.board.sticks().len())
        .filter(|k| !r.board.sticks()[*k].out && r.board.blocked(*k).is_none())
        .collect();
    *free
        .iter()
        .find(|k| r.board.head_face(**k) == prefer)
        .or(free.first())
        .expect("a stick that can go")
}

/// Turn the die so stick `k`'s screen is on top, lean it toward the
/// stick's arrow, and tap side screens until it's the one blinking.
fn aim<T: DisplayTarget>(run: &mut Run<T>, k: usize, held: u8) -> Face {
    let top = run.fw.rush().board.head_face(k);
    let d = run.fw.rush().board.sticks()[k].dir;
    hold_die(run, lean(top, Some(d), 22.0));
    advance(run, 0.4);
    let side = Face::ALL
        .into_iter()
        .find(|f| *f != top && *f != top.opposite() && held & 1 << f.index() == 0)
        .expect("a free side to tap");
    for _ in 0..rush::MAX_STICKS {
        if run.fw.rush().selected() == Some(k as u8) {
            return top;
        }
        tap_face(run, side, held);
    }
    panic!("stick {k} never lit");
}

fn top_tile<T: DisplayTarget>(run: &Run<T>, face: Face) -> Tile {
    let bytes = run.sim.lock().faces[face.index()].clone();
    common::tile::<T>(&bytes)
}

fn now<T: DisplayTarget>(run: &Run<T>) -> f64 {
    run.sim.lock().manual_time_ms.unwrap_or(0) as f64 / 1000.0
}

#[test]
fn tip_and_tap_slides_a_stick_off() {
    let mut run = start::<Rgb64>();
    advance(&mut run, rush::INTRO_MS as f64 / 1000.0 + 0.1);
    assert_eq!(run.fw.rush().phase, Phase::Play);
    let before = run.fw.rush().board.left();

    // Level: nothing lights, and a tap on top does nothing.
    let k = free_stick(&run, Face::PosZ);
    let top = run.fw.rush().board.head_face(k);
    hold_die(&mut run, lean(top, None, 0.0));
    advance(&mut run, 0.4);
    assert_eq!(run.fw.rush().selected(), None);
    tap_face(&mut run, top, 0);
    assert_eq!(run.fw.rush().board.left(), before);

    // A slight lean (under the threshold) still doesn't count.
    let d = run.fw.rush().board.sticks()[k].dir;
    hold_die(&mut run, lean(top, Some(d), 8.0));
    advance(&mut run, 0.4);
    assert_eq!(run.fw.rush().downhill(), None);

    let top = aim(&mut run, k, 0);
    run.sim.lock().haptics.clear();
    tap_face(&mut run, top, 0);
    assert_eq!(run.fw.rush().board.left(), before - 1);
    assert!(run.sim.lock().haptics.contains(&HapticEffect::Tick));
    advance(&mut run, 1.5);
    assert!(run.fw.rush().board.sticks()[k].gone());
}

#[test]
fn a_grip_on_the_sides_doesnt_hide_a_tap_on_top() {
    let mut run = start::<Rgb64>();
    advance(&mut run, 1.6);
    let k = free_stick(&run, Face::PosZ);
    let top = run.fw.rush().board.head_face(k);
    hold_die(&mut run, lean(top, None, 0.0));
    advance(&mut run, 0.4);
    // Fingers on the bottom and two sides, all the while.
    let grip: u8 = Face::ALL
        .into_iter()
        .filter(|f| *f != top)
        .take(3)
        .fold(0, |m, f| m | 1 << f.index());
    assert_eq!(grip & 1 << top.index(), 0);
    run.sim.lock().touch_mask = grip;
    advance(&mut run, 1.0);
    assert_eq!(*run.fw.mode(), Mode::Idle, "a long grip doesn't open the menu");
    let top = aim(&mut run, k, grip);
    let before = run.fw.rush().board.left();
    tap_face(&mut run, top, grip);
    assert_eq!(run.fw.rush().board.left(), before - 1);
}

#[test]
fn holding_the_top_screen_opens_the_menu() {
    let mut run = start::<Rgb64>();
    advance(&mut run, 1.6);
    hold_die(&mut run, lean(Face::PosY, None, 0.0));
    advance(&mut run, 0.4);
    run.sim.lock().touch_mask = 1 << Face::PosY.index();
    advance(&mut run, 1.0);
    assert_eq!(*run.fw.mode(), Mode::Menu);
    run.sim.lock().touch_mask = 0;
    advance(&mut run, 0.2);
    assert_eq!(
        *run.fw.mode(),
        Mode::Menu,
        "letting go of the hold that opened it keeps it open"
    );
}

#[test]
fn a_blocked_stick_jams_and_buzzes() {
    let mut run = start::<Rgb64>();
    advance(&mut run, 1.6);
    let r = run.fw.rush();
    let k = (0..r.board.sticks().len())
        .find(|k| r.board.blocked(*k).is_some())
        .expect("a blocked stick");
    let top = aim(&mut run, k, 0);
    run.sim.lock().haptics.clear();
    let before = run.fw.rush().board.left();
    tap_face(&mut run, top, 0);
    assert_eq!(run.fw.rush().jams, 1);
    assert_eq!(run.fw.rush().board.left(), before);
    assert!(run.sim.lock().haptics.contains(&HapticEffect::Buzz));
}

/// Clear the level, stick by stick.
fn clear_level<T: DisplayTarget>(run: &mut Run<T>) {
    while run.fw.rush().board.left() > 0 {
        let k = free_stick(run, Face::PosZ);
        let top = aim(run, k, 0);
        tap_face(run, top, 0);
        advance(run, 0.8);
    }
}

#[test]
fn clearing_the_board_goes_on_to_level_two() {
    let mut run = start::<Rgb64>();
    advance(&mut run, 1.6);
    clear_level(&mut run);
    advance(&mut run, 0.2);
    assert!(matches!(run.fw.rush().phase, Phase::Cleared { .. }));
    run.sim.lock().rng_script.push_back(SEED + 1);
    advance(&mut run, rush::CLEARED_MS as f64 / 1000.0 + 0.2);
    assert_eq!(run.fw.rush().level, 2);
    assert!(run.fw.rush().board.left() > 0);
}

#[test]
fn the_puzzle_keeps_through_the_menu() {
    let mut run = start::<Rgb64>();
    advance(&mut run, 1.6);
    let k = free_stick(&run, Face::PosZ);
    let top = aim(&mut run, k, 0);
    tap_face(&mut run, top, 0);
    advance(&mut run, 1.0);
    let board = run.fw.rush().board;
    // Into the menu and out again without saving.
    hold_die(&mut run, lean(Face::PosY, None, 0.0));
    advance(&mut run, 0.4);
    run.sim.lock().touch_mask = 1 << Face::PosY.index();
    advance(&mut run, 1.0);
    run.sim.lock().touch_mask = 0;
    assert_eq!(*run.fw.mode(), Mode::Menu);
    advance(&mut run, 26.0);
    assert_eq!(*run.fw.mode(), Mode::Idle);
    assert_eq!(run.fw.rush().board, board);
}

/// The moments on the sheet: the top face at each, and when.
fn moments<T: DisplayTarget>() -> (Vec<f64>, Vec<Tile>) {
    let mut run = start::<T>();
    let mut tiles = Vec::new();
    let mut times = Vec::new();
    macro_rules! shot {
        ($face:expr) => {{
            times.push(now(&run));
            tiles.push(top_tile(&run, $face));
        }};
    }
    let top = Face::PosY;
    advance(&mut run, 0.3);
    shot!(top); // level number
    advance(&mut run, 1.4);
    let k = free_stick(&run, top);
    let k_top = run.fw.rush().board.head_face(k);
    hold_die(&mut run, lean(k_top, None, 0.0));
    advance(&mut run, 0.4);
    shot!(k_top); // the board, level
    let k_top = aim(&mut run, k, 0);
    advance(&mut run, 0.05);
    shot!(k_top); // leaning: lit, blinking, edge
    tap_face(&mut run, k_top, 0);
    advance(&mut run, 0.1);
    shot!(k_top); // sliding off
    advance(&mut run, 1.0);
    shot!(k_top); // gone
                  // A jam: the stick it hits flashes red.
    let r = run.fw.rush();
    if let Some(j) =
        (0..r.board.sticks().len()).find(|j| !r.board.sticks()[*j].out && r.board.blocked(*j).is_some())
    {
        let j_top = aim(&mut run, j, 0);
        tap_face(&mut run, j_top, 0);
        advance(&mut run, 0.08);
        shot!(j_top);
    }
    clear_level(&mut run);
    advance(&mut run, 0.3);
    let top = run.fw.rush().board.head_face(0);
    shot!(top); // Clear!
    (times, tiles)
}

#[test]
fn contact_sheet() {
    let (_, grey) = moments::<Grey96>();
    let (times, rgb) = moments::<Rgb64>();
    assert_eq!(grey.len(), rgb.len());
    let times: &'static [f64] = Vec::leak(times);
    let played = vec![("level, board, lean, slide, gone, jam, clear", times, grey, rgb)];
    common::check_and_draw(&played, "rgb64-rush", "contact-sheet-rush", "Sugar Rush");
}
