//! Brief 3, test 1: every table and held screen, in every game, on both
//! panels, against the text tiers ([`smokebomb_core::tiers`]).
//!
//! The painter notes every text it draws (its cap height in panel px and its
//! length), so this checks what the panel really shows. A table screen fails
//! if any text is under T2, a T1 runs past 3 characters, a T2 past 5, or
//! there is more than one T1 and one T2 (a wrapped line is two). A held
//! screen fails on text under H2 or an H2 line past 10 characters. A tap's
//! hint may be held-sized on a table screen.
//!
//! Held screens are drawn directly, every menu page and value; table
//! screens come from the whole firmware playing each game on the simulator,
//! checked once each screen has settled.

mod common;

use std::mem::MaybeUninit;

use common::Run;
use smokebomb_core::display::Framebuffer;
use smokebomb_core::font::Fonts;
use smokebomb_core::gfx::{Layer, Mark, MarkKind, Painter, Transform};
use smokebomb_core::menu::{Draft, Held, Page, PlayMode, Settings, Setup};
use smokebomb_core::orientation::Quarter;
use smokebomb_core::pack::PackIndex;
use smokebomb_core::pigs::Pose::*;
use smokebomb_core::screens::Ctx;
use smokebomb_core::target::DisplayTarget;
use smokebomb_core::tiers::{self, Screen, Tiers};
use smokebomb_core::tips::TipDir;
use smokebomb_hal::{Face, Grey96, Rgb64};
use smokebomb_hal_simulator::{SimAssets, SimHandle};
use smokebomb_shared::{DieKind, ModeSet};

/// What's wrong with one screen's text, if anything.
fn check(screen: Screen, marks: &[Mark], t: &Tiers) -> Vec<String> {
    let mut bad = Vec::new();
    // Text fading in or out isn't the settled screen.
    let marks: Vec<&Mark> = marks.iter().filter(|m| m.alpha >= 0.5).collect();
    if marks.len() >= 12 {
        bad.push("too much text to count".into());
    }
    let cap = |m: &Mark| m.cap_px.round();
    for m in &marks {
        if m.width_px.round() > t.line {
            bad.push(format!("wider than a line ({} px): {m:?}", t.line));
        }
    }
    for m in marks.iter().filter(|m| m.kind == MarkKind::Hint) {
        if cap(m) < t.h2 || m.chars > tiers::H2_CHARS {
            bad.push(format!("hint {m:?}"));
        }
    }
    let text: Vec<&&Mark> = marks.iter().filter(|m| m.kind != MarkKind::Hint).collect();
    match screen {
        Screen::Table => {
            let (mut t1, mut t2) = (0, 0);
            for m in &text {
                if cap(m) < t.t2 {
                    bad.push(format!("under T2 ({} px): {m:?}", t.t2));
                } else if cap(m) >= t.t1_floor {
                    t1 += 1;
                    if m.kind == MarkKind::Text && m.chars > tiers::T1_CHARS {
                        bad.push(format!("T1 past {} characters: {m:?}", tiers::T1_CHARS));
                    }
                    if m.chars > 2 && cap(m) < t.t1_floor {
                        bad.push(format!("3-character T1 under the floor: {m:?}"));
                    }
                } else {
                    t2 += 1;
                    if m.kind == MarkKind::Text && m.chars > tiers::T2_CHARS {
                        bad.push(format!("T2 past {} characters: {m:?}", tiers::T2_CHARS));
                    }
                }
            }
            if t1 > 1 || t2 > 1 {
                bad.push(format!("{t1} T1 and {t2} T2 (one of each at most): {text:?}"));
            }
        }
        Screen::Held => {
            for m in &text {
                if cap(m) < t.h2 {
                    bad.push(format!("under H2 ({} px): {m:?}", t.h2));
                } else if cap(m) > 1.5 * t.h2 && cap(m) < t.h1 {
                    // Bigger than a label but not H1: a value too small.
                    bad.push(format!("a value under H1 ({} px): {m:?}", t.h1));
                } else if cap(m) < t.h1 && m.kind == MarkKind::Text && m.chars > tiers::H2_CHARS {
                    bad.push(format!("H2 past {} characters: {m:?}", tiers::H2_CHARS));
                }
            }
        }
        Screen::Other => {}
    }
    bad
}

// ---------- held screens, drawn directly ----------

/// Fonts and the asset pack.
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

    /// The marks `draw` leaves on a face.
    fn marks<T: DisplayTarget>(&mut self, draw: &dyn Fn(&mut Ctx<SimAssets, T>)) -> Vec<Mark> {
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
        painter.marks.to_vec()
    }
}

/// Every menu page with every value: each game's ring, setting up Pig Toss,
/// a game in play, the Settings app and the pages a game opens alone.
fn every_page() -> Vec<Draft> {
    let mut drafts = Vec::new();
    for play in PlayMode::ALL {
        let s = Settings {
            play,
            players: 6,
            ..Settings::default()
        };
        let d = Draft::new(&s);
        drafts.push(d);
        drafts.push(d.with_session(true));
        if let Held::Next(next) = d.held() {
            drafts.push(next);
        }
    }
    let apps = Draft::new(&Settings::default()).tipped(TipDir::Right);
    if let Held::Next(settings) = apps.tipped(TipDir::Down).held() {
        drafts.push(settings);
    }
    drafts.push(Draft::alone(&Settings::default(), Page::Next));
    drafts.push(Draft::alone(&Settings::default(), Page::Pot));
    let mut out = Vec::new();
    for d in drafts {
        for i in 0..d.ring().len() as i32 {
            for v in 0..34 {
                out.push(d.stepped(TipDir::Left, i).stepped(TipDir::Up, v));
            }
        }
    }
    out
}

fn held_screens<T: DisplayTarget>(t: &Tiers) -> Vec<String> {
    let mut kit = Kit::new();
    let mut bad = Vec::new();
    let mut note = |what: String, marks: Vec<Mark>| {
        for b in check(Screen::Held, &marks, t) {
            bad.push(format!("{what}: {b}"));
        }
    };
    for d in every_page() {
        let marks = kit.marks::<T>(&|c| T::draw_menu(c, &d, 0.78, 0.0, 0.0, 1.0, 1.0));
        note(format!("menu {:?} {:?}", d.page, d.value()), marks);
    }
    let previews = [("bank", "100"), ("bills", "3"), ("next", "")];
    for (word, value) in previews {
        let marks = kit.marks::<T>(&|c| T::draw_hold_preview(c, word, value));
        note(format!("preview {word}"), marks);
    }
    let setups = [
        Setup::Roll(DieKind::D20, 1),
        Setup::Roll(DieKind::D100, 10),
        Setup::Roll(DieKind::PassThePot, 3),
        Setup::HotPotato,
        Setup::Pigs(6),
    ];
    for setup in setups {
        let marks = kit.marks::<T>(&|c| T::draw_success(c, &setup.short_label(), setup.nudge(), 0.6));
        note(format!("success {setup:?}"), marks);
    }
    bad
}

#[test]
fn held_screens_on_the_64() {
    let bad = held_screens::<Rgb64>(&tiers::RGB64);
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn held_screens_on_the_96() {
    let bad = held_screens::<Grey96>(&tiers::GREY96);
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

// ---------- table screens, from the whole firmware ----------

/// Checks every face's last frame and notes what's wrong under `what`.
fn audit<T: DisplayTarget>(run: &Run<T>, t: &Tiers, what: &str, bad: &mut Vec<String>) {
    let mut table = false;
    for (face, a) in Face::ALL.iter().zip(run.fw.text_audit()) {
        table |= a.screen == Screen::Table;
        for b in check(a.screen, &a.marks, t) {
            bad.push(format!("{what}, {face:?} ({:?}): {b}", a.screen));
        }
    }
    if !table {
        bad.push(format!("{what}: no table screen showed"));
    }
}

fn game<T: DisplayTarget>(play: PlayMode, edit: impl FnOnce(&mut Settings)) -> Run<T> {
    let mut s = Settings {
        enabled: ModeSet::ALL,
        play,
        ..Settings::default()
    };
    edit(&mut s);
    let mut run = Run::<T>::new(s);
    run.advance_to(6.0);
    run
}

fn tap<T: DisplayTarget>(run: &mut Run<T>) {
    run.sim.lock().touch_mask = 1 << Face::PosZ.index();
    run.wait(0.05);
    run.sim.lock().touch_mask = 0;
}

fn hold<T: DisplayTarget>(run: &mut Run<T>) {
    run.sim.lock().touch_mask = 1 << Face::PosZ.index();
    run.wait(1.0);
    run.sim.lock().touch_mask = 0;
}

fn table_screens<T: DisplayTarget>(t: &Tiers) -> Vec<String> {
    let mut bad = Vec::new();

    // Dice: the setup label, then results (plain, several dice, max, dud).
    for (die, count) in [(DieKind::D20, 1), (DieKind::D6, 3), (DieKind::D100, 10)] {
        let mut run = game::<T>(PlayMode::Dice, |s| {
            s.die = die;
            s.count = count;
        });
        tap(&mut run);
        run.wait(0.8);
        audit(&run, t, &format!("label {count}{die:?}"), &mut bad);
    }
    for (die, count, words, what) in [
        (DieKind::D20, 1, vec![10u32], "d20: 11"),
        (DieKind::D6, 3, vec![2, 4, 1], "3d6"),
        (DieKind::D20, 1, vec![19], "d20 max"),
        (DieKind::D20, 1, vec![0], "d20 dud"),
        (DieKind::D100, 10, vec![99; 10], "10d100 max"),
        (DieKind::D100, 10, vec![50; 10], "10d100"),
    ] {
        let mut run = game::<T>(PlayMode::Dice, |s| {
            s.die = die;
            s.count = count;
        });
        run.throw_with(&words);
        run.wait(7.0);
        audit(&run, t, what, &mut bad);
    }

    // Pass the Pot: the bills screen, and results.
    let mut run = game::<T>(PlayMode::PassThePot, |_| {});
    tap(&mut run);
    run.wait(0.8);
    audit(&run, t, "pot bills", &mut bad);
    for (n, words, what) in [
        (3, vec![0u32, 2, 1], "pot left right pot"),
        (2, vec![3, 5], "pot keep all"),
        (1, vec![1], "pot one to the pot"),
    ] {
        let mut run = game::<T>(PlayMode::PassThePot, |s| s.pot_count = n);
        run.throw_with(&words);
        run.wait(7.0);
        audit(&run, t, what, &mut bad);
    }

    // Hot Potato: the label, the fuse, the boom.
    let mut run = game::<T>(PlayMode::HotPotato, |_| {});
    tap(&mut run);
    run.wait(0.8);
    audit(&run, t, "potato label", &mut bad);
    run.wait(4.0);
    run.sim.lock().rng_script.push_back(0);
    run.shake();
    run.wait(4.0);
    audit(&run, t, "potato fuse", &mut bad);
    run.wait(17.5);
    audit(&run, t, "potato boom", &mut bad);

    // Pig Toss: the label, a score, an oops, a smooch, a rare throw, a bank
    // locking in, and a win.
    let mut run = game::<T>(PlayMode::PigToss, |_| {});
    tap(&mut run);
    run.wait(0.8);
    audit(&run, t, "pigs label", &mut bad);
    for (poses, touching, what) in [
        ([Back, Feet], false, "pigs score"),
        ([SideDot, SidePlain], false, "pigs oops"),
        ([Feet, Back], true, "pigs smooch"),
        ([Nose, Ear], false, "pigs rare"),
    ] {
        let mut run = game::<T>(PlayMode::PigToss, |_| {});
        run.throw_pigs(poses, touching);
        run.wait(5.0);
        audit(&run, t, what, &mut bad);
    }
    let mut run = game::<T>(PlayMode::PigToss, |_| {});
    run.throw_pigs([Ear, Ear], false);
    run.wait(5.0);
    tap(&mut run);
    run.wait(0.5);
    audit(&run, t, "pigs hint", &mut bad);
    run.wait(3.0);
    hold(&mut run);
    run.wait(4.0);
    audit(&run, t, "pigs lock-in", &mut bad);
    run.wait(10.0);
    run.throw_pigs([Back, Feet], false);
    run.wait(5.0);
    hold(&mut run);
    run.wait(12.0);
    run.throw_pigs([Nose, Nose], false);
    run.wait(6.0);
    assert!(run.fw.pigs().winner().is_some(), "the win to check");
    audit(&run, t, "pigs win", &mut bad);
    bad
}

#[test]
fn table_screens_on_the_64() {
    let bad = table_screens::<Rgb64>(&tiers::RGB64);
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn table_screens_on_the_96() {
    let bad = table_screens::<Grey96>(&tiers::GREY96);
    assert!(bad.is_empty(), "{} problems:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn the_check_catches_what_the_brief_rules_out() {
    let t = &tiers::RGB64;
    let m = |cap_px: f32, chars: u8| Mark {
        cap_px,
        width_px: 10.0,
        chars,
        alpha: 1.0,
        kind: MarkKind::Text,
    };
    assert!(check(Screen::Table, &[m(30.0, 2), m(15.0, 5)], t).is_empty());
    assert!(
        !check(Screen::Table, &[m(30.0, 4)], t).is_empty(),
        "a 4-character T1"
    );
    assert!(
        !check(Screen::Table, &[m(15.0, 6)], t).is_empty(),
        "a 6-character T2"
    );
    assert!(!check(Screen::Table, &[m(7.0, 3)], t).is_empty(), "small text");
    assert!(
        !check(Screen::Table, &[m(15.0, 3), m(15.0, 3)], t).is_empty(),
        "a wrapped T2"
    );
    assert!(check(Screen::Held, &[m(7.0, 10), m(18.0, 4)], t).is_empty());
    assert!(
        !check(Screen::Held, &[m(7.0, 11)], t).is_empty(),
        "an 11-character H2"
    );
    assert!(!check(Screen::Held, &[m(5.0, 3)], t).is_empty(), "under H2");
    assert!(
        !check(Screen::Held, &[m(14.0, 3)], t).is_empty(),
        "a value under H1"
    );
    let wide = Mark {
        width_px: 70.0,
        ..m(15.0, 5)
    };
    assert!(!check(Screen::Table, &[wide], t).is_empty(), "too wide");
}
