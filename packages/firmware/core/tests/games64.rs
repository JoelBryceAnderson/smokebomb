//! Hot Potato, Pass the Pot and the hold ring on the 30 mm die's 64×64
//! panels next to the 34 mm die's 96×96, from the whole firmware: each
//! scenario plays on the simulator, once per target, with the same inputs.
//!
//! * `tests/snapshots/rgb64-games.png`: every 64×64 checkpoint at 1:1,
//!   compared exactly (`UPDATE_SNAPSHOTS=1` regenerates it).
//! * `docs/30mm/contact-sheet-games.png` (with `UPDATE_SNAPSHOTS=1`, and
//!   always in `target/`): per scenario, the 64×64 face at 4× over the
//!   96×96 face at 3×.

mod common;

use smokebomb_core::menu::{PlayMode, Settings};
use smokebomb_shared::ModeSet;

const THROW_AT: f64 = 10.4;

fn with(play: PlayMode, pot_count: u8) -> Settings {
    Settings {
        enabled: ModeSet::ALL,
        play,
        pot_count,
        ..Settings::default()
    }
}

fn hot_potato() -> Settings {
    with(PlayMode::HotPotato, 3)
}

fn pot(n: u8) -> Settings {
    with(PlayMode::PassThePot, n)
}

fn dice() -> Settings {
    with(PlayMode::Dice, 3)
}

fn scenarios() -> Vec<common::Scenario> {
    const POT_AFTER: &[f64] = &[11.0, 11.6, 12.2, 13.0, 15.0];
    vec![
        scenario!("potato: label", hot_potato, &[7.0, 7.5], [(6.6, |r| r.tap())]),
        scenario!(
            "potato: fuse and boom",
            hot_potato,
            &[10.5, 18.0, 26.0, 30.0, 34.0, 36.0, 38.0, 39.0],
            [(9.0, |r| {
                // A 15 s fuse.
                r.sim.lock().rng_script.push_back(1 << 31);
                r.shake()
            })]
        ),
        scenario!(
            "potato: boom",
            hot_potato,
            &[39.1, 39.15, 39.25, 39.35, 39.5, 39.7, 40.0, 40.5, 41.5],
            [(9.0, |r| {
                r.sim.lock().rng_script.push_back(1 << 31);
                r.shake()
            })]
        ),
        // A tap shows the bills and what a hold does; the hold shows its
        // action as the ring fills, then opens Bills alone to change them.
        scenario!(
            "pot: bills, hold to change",
            || pot(3),
            &[7.0, 7.4, 7.8, 8.2, 8.6, 9.0],
            [(6.6, |r| r.tap()), (7.3, |r| r.press()), (8.4, |r| r.release()),]
        ),
        // Raw d6 values are word % 6 + 1: 1 ←, 2 pot, 3 →, 4–6 keep.
        scenario!(
            "pot x3: left, right, pot",
            || pot(3),
            POT_AFTER,
            [(THROW_AT, |r| r.throw_with(&[0, 2, 1]))]
        ),
        scenario!(
            "pot x3: two left, keep",
            || pot(3),
            POT_AFTER,
            [(THROW_AT, |r| r.throw_with(&[0, 0, 4]))]
        ),
        scenario!(
            "pot x2: keep all",
            || pot(2),
            POT_AFTER,
            [(THROW_AT, |r| r.throw_with(&[3, 5]))]
        ),
        scenario!(
            "pot x1: pot",
            || pot(1),
            POT_AFTER,
            [(THROW_AT, |r| r.throw_with(&[1]))]
        ),
        scenario!(
            "hold ring",
            dice,
            &[9.15, 9.35, 9.55, 9.75, 9.95, 10.2, 10.5],
            [(9.0, |r| r.press()), (10.4, |r| r.release())]
        ),
    ]
}

#[test]
fn games_on_both_panels() {
    let played = common::play_both(scenarios());
    common::check_and_draw(
        &played,
        "rgb64-games",
        "contact-sheet-games",
        "Hot Potato, Pass the Pot, hold ring: 30 mm 64x64 (4x, top) vs 34 mm 96x96 (3x, below)",
    );
}
