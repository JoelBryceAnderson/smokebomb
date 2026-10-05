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

// ---------- holds that commit, taps that don't (brief 3, part 2) ----------

use smokebomb_core::menu::{Page, PlayMode};
use smokebomb_core::pigs::Pose::{self, *};
use smokebomb_core::potato::PotatoState;
use smokebomb_hal::HapticEffect;

fn playing(play: PlayMode) -> Run<Rgb64> {
    let mut run = Run::new(Settings {
        play,
        ..Settings::default()
    });
    // Past the boot, with its label gone.
    run.advance_to(6.0);
    run
}

/// Everything a tap must leave alone, as text. A lit fuse's ticks move on
/// by themselves, so only its lighting and length count.
fn game_state(run: &Run<Rgb64>) -> String {
    let potato = match *run.fw.potato().state() {
        PotatoState::Lit {
            since_ms, fuse_ms, ..
        } => format!("lit {since_ms} {fuse_ms}"),
        s => format!("{s:?}"),
    };
    format!(
        "{:?} {potato} {:?} {:?}",
        run.fw.pigs(),
        run.fw.settings(),
        run.fw.last_roll()
    )
}

/// A tap on each face in turn, each on a die that has been resting.
fn tap_every_face(run: &mut Run<Rgb64>) {
    for face in Face::ALL {
        touch(run, 1 << face.index());
        run.wait(0.05);
        touch(run, 0);
        run.wait(0.6);
    }
}

/// The die lands, the pigs settle, and the result stays up.
fn throw_pigs(run: &mut Run<Rgb64>, poses: [Pose; 2]) {
    run.throw_pigs(poses, false);
    run.wait(4.0);
}

fn hold(run: &mut Run<Rgb64>, face: Face) {
    touch(run, 1 << face.index());
    run.wait(1.0);
    touch(run, 0);
    run.wait(0.3);
}

fn saves(run: &Run<Rgb64>) -> usize {
    run.sim
        .lock()
        .haptics
        .iter()
        .filter(|h| **h == HapticEffect::MenuSave)
        .count()
}

#[test]
fn taps_change_no_game_state_in_any_mode() {
    // Test 3: every face, in each mode's states between throws.
    fn check(run: &mut Run<Rgb64>, what: &str) {
        let before = game_state(run);
        tap_every_face(run);
        assert_eq!(game_state(run), before, "{what}");
        assert_ne!(*run.fw.mode(), Mode::Menu, "{what}");
    }

    let mut dice = playing(PlayMode::Dice);
    check(&mut dice, "dice, idle");
    dice.throw_with(&[7]);
    dice.wait(4.0);
    check(&mut dice, "dice, a result");

    let mut pot = playing(PlayMode::PassThePot);
    check(&mut pot, "pot, idle and its bills screen");
    pot.throw_with(&[0, 2, 1]);
    pot.wait(4.0);
    check(&mut pot, "pot, a result");

    let mut potato = playing(PlayMode::HotPotato);
    check(&mut potato, "potato, idle");
    potato.sim.lock().rng_script.push_back(0);
    potato.shake();
    potato.wait(2.0);
    assert!(potato.fw.potato().is_lit());
    check(&mut potato, "potato, lit");
    potato.wait(16.0);
    assert!(potato.fw.potato().boomed_for(u64::MAX).is_some(), "it went off");
    let before = game_state(&potato);
    touch(&mut potato, 1 << Face::PosZ.index());
    potato.wait(0.05);
    touch(&mut potato, 0);
    potato.wait(0.1);
    assert_eq!(
        game_state(&potato),
        before,
        "potato, spent: a tap doesn't reset it"
    );

    let mut pigs = playing(PlayMode::PigToss);
    check(&mut pigs, "pigs, a new game");
    throw_pigs(&mut pigs, [Back, Back]);
    check(&mut pigs, "pigs, a turn with points");
}

#[test]
fn a_hold_banks_once_with_the_commit_haptic() {
    // Test 6 (Pig Toss) and test 8: with an action pending, a hold does it
    // instead of opening the menu.
    let mut run = playing(PlayMode::PigToss);
    throw_pigs(&mut run, [Back, Back]);
    assert_eq!(run.fw.pigs().turn(), 20);
    let before = saves(&run);
    hold(&mut run, Face::PosZ);
    assert_eq!(run.fw.pigs().scores(), &[20, 0]);
    assert_eq!(run.fw.pigs().current(), 1);
    assert_eq!(saves(&run), before + 1);
    assert_ne!(*run.fw.mode(), Mode::Menu);
    // Nothing pending now: the next hold opens the menu.
    touch(&mut run, 1 << Face::PosZ.index());
    run.wait(1.0);
    assert_eq!(*run.fw.mode(), Mode::Menu);
}

#[test]
fn a_hold_on_the_bills_screen_opens_bills_alone() {
    // Test 6 (Pass the Pot): the count changes in the adjust page, and only
    // when it saves.
    let mut run = playing(PlayMode::PassThePot);
    touch(&mut run, 1 << Face::PosZ.index());
    run.wait(0.05);
    touch(&mut run, 0);
    run.wait(0.4);
    touch(&mut run, 1 << Face::PosZ.index());
    run.wait(1.0);
    let draft = run.fw.menu_draft().expect("Bills opened");
    assert_eq!(draft.page, Page::Pot);
    assert_eq!(draft.ring(), &[Page::Pot]);
    touch(&mut run, 0);
    run.wait(0.5);
    // A hold saves as it was; the count is still three.
    hold(&mut run, Face::PosZ);
    assert!(run.fw.menu_draft().is_none());
    assert_eq!(run.fw.settings().pot_count, 3);
}

#[test]
fn after_a_win_a_hold_offers_a_rematch() {
    let mut run = playing(PlayMode::PigToss);
    // Player A throws 60 and banks, B throws 10 and banks, A throws 40.
    throw_pigs(&mut run, [Ear, Ear]);
    hold(&mut run, Face::PosZ);
    throw_pigs(&mut run, [Back, Feet]);
    hold(&mut run, Face::PosZ);
    throw_pigs(&mut run, [Nose, Nose]);
    assert_eq!(run.fw.pigs().winner(), Some(0));
    // The win screen is up: a hold opens Next on Rematch.
    run.wait(2.0);
    touch(&mut run, 1 << Face::PosZ.index());
    run.wait(1.0);
    let draft = run.fw.menu_draft().expect("Next opened");
    assert_eq!(draft.page, Page::Next);
    assert_eq!(draft.value().as_str(), "Again");
    touch(&mut run, 0);
    run.wait(0.5);
    // A hold there starts the same table again from 0.
    hold(&mut run, Face::PosZ);
    assert!(run.fw.menu_draft().is_none());
    assert_eq!(run.fw.pigs().winner(), None);
    assert_eq!(run.fw.pigs().scores(), &[0, 0]);
}
