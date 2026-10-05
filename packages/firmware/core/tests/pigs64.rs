//! Pig Toss on the 30 mm die's 64×64 panels next to the 34 mm die's 96×96,
//! from the whole firmware: each scenario plays a real game on the
//! simulator, once per target, with the same scripted throws.
//!
//! * `tests/snapshots/rgb64-pigs.png`: every 64×64 checkpoint at 1:1,
//!   compared exactly (`UPDATE_SNAPSHOTS=1` regenerates it).
//! * `docs/30mm/contact-sheet-pigs.png` (with `UPDATE_SNAPSHOTS=1`, and
//!   always in `target/`): per scenario, the 64×64 face at 4× over the
//!   96×96 face at 3× (about the same shown size), checkpoint by checkpoint.

mod common;

use smokebomb_core::menu::{PlayMode, Settings};
use smokebomb_core::pigs::Pose;
use smokebomb_shared::ModeSet;

const THROW_AT: f64 = 10.4;

fn pig_toss() -> Settings {
    Settings {
        enabled: ModeSet::ALL,
        play: PlayMode::PigToss,
        ..Settings::default()
    }
}

fn scenarios() -> Vec<common::Scenario> {
    use Pose::*;
    const AFTER: &[f64] = &[10.9, 11.5, 12.2, 12.7, 13.1, 13.6, 14.3, 15.6, 17.0];
    vec![
        scenario!("wake label", pig_toss, &[7.0, 7.5], [(6.6, |r| r.tap())]),
        scenario!(
            "score: Belly Up + Strut",
            pig_toss,
            AFTER,
            [(THROW_AT, |r| r.throw_pigs([Back, Feet], false))]
        ),
        scenario!(
            "bust",
            pig_toss,
            AFTER,
            [(THROW_AT, |r| r.throw_pigs([SideDot, SidePlain], false))]
        ),
        scenario!(
            "smooch",
            pig_toss,
            AFTER,
            [(THROW_AT, |r| r.throw_pigs([Feet, Back], true))]
        ),
        scenario!(
            "rare: Nose Dive + Tipsy",
            pig_toss,
            AFTER,
            [(THROW_AT, |r| r.throw_pigs([Nose, Ear], false))]
        ),
        scenario!(
            "bank: lock-in",
            pig_toss,
            &[15.5, 15.75, 16.0, 16.3, 16.7, 17.3, 18.3, 20.0],
            [
                (THROW_AT, |r| r.throw_pigs([Nose, Back], false)),
                (14.8, |r| r.press()),
                (15.8, |r| r.release())
            ]
        ),
        scenario!(
            "win",
            pig_toss,
            &[30.9, 31.8, 32.3, 32.5, 32.8, 33.3, 34.5],
            [
                (THROW_AT, |r| r.throw_pigs([Ear, Ear], false)),
                (14.8, |r| r.press()),
                (15.8, |r| r.release()),
                (19.0, |r| r.throw_pigs([Back, Feet], false)),
                (23.8, |r| r.press()),
                (24.8, |r| r.release()),
                (28.0, |r| r.throw_pigs([Nose, Nose], false)),
            ]
        ),
    ]
}

#[test]
fn pig_toss_on_both_panels() {
    let played = common::play_both(scenarios());
    common::check_and_draw(
        &played,
        "rgb64-pigs",
        "contact-sheet-pigs",
        "Pig Toss: 30 mm 64x64 (4x, top) vs 34 mm 96x96 (3x, below)",
    );
}
