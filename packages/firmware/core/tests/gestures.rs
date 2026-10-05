//! Deliberate gestures on the whole firmware (brief 3, part 2): a grip, a
//! hold let go early and a shake mid-hold change nothing.

mod common;

use common::Run;
use smokebomb_core::menu::Settings;
use smokebomb_core::state::Mode;
use smokebomb_hal::{Face, Rgb64};

const Z: u8 = 1 << Face::PosZ.index();
const X: u8 = 1 << Face::PosX.index();

/// A resting Dice die, a second in.
fn die() -> Run<Rgb64> {
    let mut run = Run::new(Settings::default());
    run.advance_to(1.0);
    run
}

fn touch(run: &mut Run<Rgb64>, mask: u8) {
    run.sim.lock().touch_mask = mask;
}

#[test]
fn a_hold_on_one_face_opens_the_menu() {
    let mut run = die();
    touch(&mut run, Z);
    run.advance_to(2.0);
    assert_eq!(*run.fw.mode(), Mode::Menu);
}

#[test]
fn a_grip_opens_no_menu() {
    // Test 4: two faces for 2 s.
    let mut run = die();
    let before = *run.fw.settings();
    touch(&mut run, Z | X);
    run.advance_to(3.0);
    touch(&mut run, 0);
    run.advance_to(3.5);
    assert_eq!(*run.fw.mode(), Mode::Idle);
    assert_eq!(*run.fw.settings(), before);
}

#[test]
fn letting_go_at_0_7_s_opens_no_menu() {
    // Test 5: the hold is cancelled.
    let mut run = die();
    touch(&mut run, Z);
    run.advance_to(1.7);
    touch(&mut run, 0);
    run.advance_to(3.0);
    assert_eq!(*run.fw.mode(), Mode::Idle);
}

#[test]
fn a_shake_mid_hold_rolls_and_opens_no_menu() {
    // Test 7: the shake cancels the hold and starts a roll. The finger stays
    // on after the die is put down, and still opens nothing until lifted.
    let mut run = die();
    touch(&mut run, Z);
    run.advance_to(1.5);
    run.shake();
    run.advance_to(5.0);
    assert!(
        matches!(run.fw.mode(), Mode::Reveal { .. }),
        "{:?}",
        run.fw.mode()
    );
    assert!(run.fw.last_roll().is_some());
    touch(&mut run, 0);
    run.advance_to(5.5);
    assert!(
        matches!(run.fw.mode(), Mode::Reveal { .. }),
        "{:?}",
        run.fw.mode()
    );
}
