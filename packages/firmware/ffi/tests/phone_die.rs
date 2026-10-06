//! The die as the phone drives it: one IMU reading and touch mask a tick,
//! frames out. These play a throw the way the AR viewer makes one (picked
//! up, shaken, released, landed) and check the firmware takes it as a roll.

use smokebomb_ffi::*;

const UP_Y: SbImu = SbImu {
    accel_mg: [0, 1000, 0],
    gyro_mdps: [0; 3],
};
const UP_X: SbImu = SbImu {
    accel_mg: [1000, 0, 0],
    gyro_mdps: [0; 3],
};

fn run(die: &mut SbDie, imu: SbImu, seconds: f64) -> u64 {
    let mut seq = 0;
    for _ in 0..(seconds * 60.0).round() as u32 {
        seq = die.tick(imu.into(), 0);
    }
    seq
}

fn lit_pixels(die: &SbDie, face: usize) -> usize {
    let side = die.side() as usize;
    let mut rgba = vec![0u8; side * side * 4];
    assert!(die.face_rgba(face, &mut rgba));
    rgba.chunks_exact(4)
        .filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0)
        .count()
}

/// Lift, shake, release, fall, land: what the viewer's finite-difference IMU
/// reads during a throw, sample by sample.
fn throw(die: &mut SbDie) {
    // Picked up: a push up and a turn.
    for _ in 0..10 {
        die.tick(
            SbImu {
                accel_mg: [60, 1250, 40],
                gyro_mdps: [40_000, 0, 0],
            }
            .into(),
            0,
        );
    }
    // Shaken: well over 0.7 g off 1 g, back and forth.
    for i in 0..30 {
        let s = if i % 2 == 0 { 1 } else { -1 };
        die.tick(
            SbImu {
                accel_mg: [s * 1500, 1000, s * 600],
                gyro_mdps: [300_000, 0, 0],
            }
            .into(),
            0,
        );
    }
    // Released: free fall reads near zero.
    for _ in 0..12 {
        die.tick(
            SbImu {
                accel_mg: [20, -30, 10],
                gyro_mdps: [900_000, 200_000, 0],
            }
            .into(),
            0,
        );
    }
    // The table.
    die.tick(
        SbImu {
            accel_mg: [3000, 1500, 0],
            gyro_mdps: [200_000, 0, 0],
        }
        .into(),
        0,
    );
    // Tumbling to a stop, +X up.
    for _ in 0..10 {
        die.tick(
            SbImu {
                accel_mg: [1100, 300, 0],
                gyro_mdps: [100_000, 0, 0],
            }
            .into(),
            0,
        );
    }
}

#[test]
fn both_panels_boot_and_draw() {
    for (panel, side) in [(SB_PANEL_GREY96, 96), (SB_PANEL_RGB64, 64)] {
        let mut die = SbDie::new(panel).expect("boots");
        assert_eq!(die.side(), side);
        let seq = run(&mut die, UP_Y, 8.0);
        assert!(seq > 0, "panel {panel}: frames flushed");
        let lit: usize = (0..6).map(|f| lit_pixels(&die, f)).sum();
        assert!(lit > 0, "panel {panel}: something is on the screens after boot");
        assert_eq!(die.mode(), "Idle", "panel {panel}: idle after the boot animation");
    }
}

#[test]
fn a_viewer_style_throw_is_a_roll() {
    for panel in [SB_PANEL_GREY96, SB_PANEL_RGB64] {
        let mut die = SbDie::new(panel).unwrap();
        run(&mut die, UP_Y, 8.0);
        throw(&mut die);
        run(&mut die, UP_X, 1.5);
        assert!(
            die.mode().starts_with("Reveal"),
            "panel {panel}: mode is {}",
            die.mode()
        );
        // The result shows on the face that's up.
        assert!(lit_pixels(&die, 0) > 0, "panel {panel}: +X shows the result");
    }
}

#[test]
fn rejects_bad_arguments() {
    assert!(SbDie::new(7).is_none());
    let die = SbDie::new(SB_PANEL_RGB64).unwrap();
    let mut small = [0u8; 16];
    assert!(!die.face_rgba(0, &mut small));
    let mut big = vec![0u8; 64 * 64 * 4];
    assert!(!die.face_rgba(6, &mut big));
}

#[test]
fn c_abi_round_trip() {
    unsafe {
        assert!(sb_die_new(9).is_null());
        let die = sb_die_new(SB_PANEL_RGB64);
        assert!(!die.is_null());
        assert_eq!(sb_die_panel_side(die), 64);
        let imu = UP_Y;
        for _ in 0..30 {
            sb_die_tick(die, imu, 0);
        }
        let mut rgba = vec![0u8; 64 * 64 * 4];
        assert!(sb_die_face_rgba(die, 2, rgba.as_mut_ptr(), rgba.len()));
        let mut mode = [0u8; 4];
        let n = sb_die_mode(die, mode.as_mut_ptr(), mode.len());
        assert!(n >= 3);
        assert_eq!(mode[3], 0, "NUL-terminated when truncated");
        while sb_die_next_haptic(die) >= 0 {}
        sb_die_free(die);
        sb_die_free(std::ptr::null_mut());
        assert_eq!(sb_die_tick(std::ptr::null_mut(), imu, 0), 0);
    }
}

#[test]
fn header_matches() {
    let h = include_str!("../include/smokebomb_ffi.h");
    assert!(h.contains(&format!("SB_PANEL_GREY96 = {SB_PANEL_GREY96}")));
    assert!(h.contains(&format!("SB_PANEL_RGB64 = {SB_PANEL_RGB64}")));
    assert!(h.contains(&format!("#define SB_TICK_HZ {}", smokebomb_core::TICK_HZ)));
    assert!(h.contains(&format!("#define SB_FACE_COUNT {}", smokebomb_hal::FACE_COUNT)));
    for f in [
        "sb_die_new",
        "sb_die_free",
        "sb_die_panel_side",
        "sb_die_tick",
        "sb_die_face_rgba",
        "sb_die_next_haptic",
        "sb_die_mode",
        "sb_die_set_local_time",
        "sb_die_phone_connect",
        "sb_die_phone_send",
        "sb_die_phone_receive",
    ] {
        assert!(h.contains(&format!("{f}(")), "{f} declared");
    }
    assert_eq!(
        std::mem::size_of::<SbImu>(),
        20,
        "SbImu: three i16, padding, three i32"
    );
}

/// The AR viewer's throw, rebuilt from positions as the viewer makes it:
/// the windup (DiePhysics.windup: eased lift, then a 1 − cos shake), a
/// ballistic flight, a landing, a tumble to rest. Readings come from the
/// same finite differences as ImuSynth.swift.
#[test]
fn the_viewers_windup_and_flight_read_as_a_roll() {
    const DT: f32 = 1.0 / 60.0;
    const G: f32 = 9.80665;
    let windup = |t: f32| -> [f32; 3] {
        let u = (t / 0.3).clamp(0.0, 1.0);
        let mut p = [0.0, 0.05 * u * u * (3.0 - 2.0 * u), 0.0];
        if t > 0.3 {
            let ts = t - 0.3;
            let amp = [0.0075, 0.002, 0.0057];
            let freq = [6.5, 7.0, 7.5];
            for i in 0..3 {
                p[i] += amp[i] * (1.0 - (std::f32::consts::TAU * freq[i] * ts).cos());
            }
        }
        p
    };

    // Positions, one per tick: rest, windup, flight, landing, rest.
    let mut path: Vec<[f32; 3]> = vec![[0.0; 3]; 30];
    for k in 0..48 {
        path.push(windup(k as f32 * DT));
    }
    let start = *path.last().unwrap();
    let (vx, mut vy) = (0.8_f32, 0.5_f32);
    let mut p = start;
    loop {
        vy -= G * DT;
        p = [p[0] + vx * DT, p[1] + vy * DT, p[2]];
        if p[1] <= 0.0 {
            p[1] = 0.0;
            break;
        }
        path.push(p);
    }
    path.push(p); // landed: the fall stops dead on the table
    for _ in 0..60 {
        path.push(p);
    }

    for panel in [SB_PANEL_GREY96, SB_PANEL_RGB64] {
        let mut die = SbDie::new(panel).unwrap();
        run(&mut die, UP_Y, 8.0);
        let mut saw_shaking = false;
        let mut saw_airborne = false;
        for k in 0..path.len() {
            let mut accel = [0.0, 1000.0, 0.0];
            if k >= 2 {
                for i in 0..3 {
                    accel[i] += (path[k][i] - 2.0 * path[k - 1][i] + path[k - 2][i]) / (DT * DT) / G * 1000.0;
                }
            }
            let reading = SbImu {
                accel_mg: accel.map(|a| a.round().clamp(-32768.0, 32767.0) as i16),
                gyro_mdps: [0; 3],
            };
            die.tick(reading.into(), 0);
            saw_shaking |= die.mode() == "Shaking";
            saw_airborne |= die.mode() == "Airborne";
        }
        run(&mut die, UP_Y, 1.5);
        assert!(saw_shaking, "panel {panel}: the windup reads as shaking");
        assert!(saw_airborne, "panel {panel}: the release reads as airborne");
        assert!(
            die.mode().starts_with("Reveal"),
            "panel {panel}: mode is {}",
            die.mode()
        );
    }
}

/// A finger held on a resting die opens the menu; it reports the face it's
/// read on until it closes.
#[test]
fn a_long_press_opens_the_menu_and_names_its_front() {
    let mut die = SbDie::new(SB_PANEL_RGB64).unwrap();
    run(&mut die, UP_Y, 8.0);
    assert_eq!(unsafe { sb_die_menu_front(&die) }, -1, "closed at first");
    for _ in 0..120 {
        die.tick(UP_Y.into(), 1 << 4); // holding +Z
    }
    for _ in 0..10 {
        die.tick(UP_Y.into(), 0);
    }
    assert_eq!(die.mode(), "Menu");
    let front = unsafe { sb_die_menu_front(&die) };
    assert!((0..6).contains(&front), "menu front is {front}");
}

/// AR off: the die locked in place. A small windup (1.5 cm) and a toss
/// straight up (0.25 m/s) still read as a roll: picked up, shaken, airborne
/// for a few ticks, a hard landing.
#[test]
fn the_locked_in_place_toss_reads_as_a_roll() {
    const DT: f32 = 1.0 / 60.0;
    const G: f32 = 9.80665;
    let windup = |t: f32| -> [f32; 3] {
        let u = (t / 0.3).clamp(0.0, 1.0);
        let mut p = [0.0, 0.015 * u * u * (3.0 - 2.0 * u), 0.0];
        if t > 0.3 {
            let ts = t - 0.3;
            let amp = [0.0075, 0.002, 0.0057];
            let freq = [6.5, 7.0, 7.5];
            for i in 0..3 {
                p[i] += amp[i] * (1.0 - (std::f32::consts::TAU * freq[i] * ts).cos());
            }
        }
        p
    };
    let mut path: Vec<[f32; 3]> = vec![[0.0; 3]; 30];
    for k in 0..48 {
        path.push(windup(k as f32 * DT));
    }
    let mut p = *path.last().unwrap();
    let mut vy = 0.25_f32;
    loop {
        vy -= G * DT;
        p[1] += vy * DT;
        if p[1] <= 0.0 {
            p[1] = 0.0;
            break;
        }
        path.push(p);
    }
    for _ in 0..61 {
        path.push(p);
    }
    for panel in [SB_PANEL_GREY96, SB_PANEL_RGB64] {
        let mut die = SbDie::new(panel).unwrap();
        run(&mut die, UP_Y, 8.0);
        let mut saw_airborne = false;
        for k in 0..path.len() {
            let mut accel = [0.0, 1000.0, 0.0];
            if k >= 2 {
                for i in 0..3 {
                    accel[i] += (path[k][i] - 2.0 * path[k - 1][i] + path[k - 2][i]) / (DT * DT) / G * 1000.0;
                }
            }
            let reading = SbImu {
                accel_mg: accel.map(|a| a.round().clamp(-32768.0, 32767.0) as i16),
                gyro_mdps: [0; 3],
            };
            die.tick(reading.into(), 0);
            saw_airborne |= die.mode() == "Airborne";
        }
        run(&mut die, UP_Y, 1.5);
        assert!(saw_airborne, "panel {panel}: the toss reads as airborne");
        assert!(
            die.mode().starts_with("Reveal"),
            "panel {panel}: mode is {}",
            die.mode()
        );
    }
}

fn drain(die: &mut SbDie) -> Vec<String> {
    std::iter::from_fn(|| die.phone_receive()).collect()
}

/// The app's link to the die on the phone: the same JSON as the desktop
/// simulator's `/phone` WebSocket.
#[test]
fn the_app_connects_sets_the_die_up_and_hears_its_rolls() {
    let mut die = SbDie::new(SB_PANEL_RGB64).unwrap();
    run(&mut die, UP_Y, 8.0);
    assert!(drain(&mut die).is_empty(), "nothing for an app that isn't there");

    die.phone_connect(true);
    let greeting = drain(&mut die);
    assert_eq!(greeting.len(), 2);
    assert!(
        greeting[0].starts_with(r#"{"Hello":{"firmware_version":"#),
        "{}",
        greeting[0]
    );
    assert_eq!(
        greeting[1],
        r#"{"Inventory":{"licensed":31,"enabled":31,"active":"Dice"}}"#
    );

    // Settings from the app, each answered with the inventory.
    assert!(die.phone_send(r#"{"SetEnabledModes":4}"#));
    assert_eq!(
        drain(&mut die),
        [r#"{"Inventory":{"licensed":31,"enabled":5,"active":"Dice"}}"#],
        "Dice stays on"
    );
    assert!(die.phone_send(r#"{"SetDie":{"kind":"D6","count":2}}"#));
    assert_eq!(drain(&mut die).len(), 1);
    assert!(!die.phone_send("not json"));

    // A throw: the roll goes to the app, with the dice it set.
    throw(&mut die);
    run(&mut die, UP_X, 1.5);
    let sent = drain(&mut die);
    let roll = sent
        .iter()
        .find(|m| m.starts_with(r#"{"Roll":"#))
        .expect("the roll is sent");
    assert!(roll.contains("D6"), "the dice the app set: {roll}");

    // And it's kept for a history sync.
    assert!(die.phone_send(r#"{"SyncHistory":{"since_counter":0}}"#));
    let history = drain(&mut die);
    assert_eq!(history.len(), 2, "{history:?}");
    assert!(history[0].starts_with(r#"{"HistoryItem":{"#));
    assert_eq!(history[1], r#"{"HistoryItem":null}"#);

    die.phone_connect(false);
    assert!(die.phone_send(r#""GetInventory""#), "still a message");
    assert!(drain(&mut die).is_empty(), "but nobody to answer");
}

#[test]
fn c_abi_phone_link() {
    unsafe {
        let die = sb_die_new(SB_PANEL_GREY96);
        sb_die_phone_connect(die, true);
        // Ask how long the next message is, then take it.
        let n = sb_die_phone_receive(die, std::ptr::null_mut(), 0);
        assert!(n > 0);
        let mut small = vec![0u8; n];
        assert_eq!(
            sb_die_phone_receive(die, small.as_mut_ptr(), small.len()),
            n,
            "too small: it stays"
        );
        let mut buf = vec![0u8; n + 1];
        assert_eq!(sb_die_phone_receive(die, buf.as_mut_ptr(), buf.len()), n);
        assert_eq!(buf[n], 0);
        assert!(std::str::from_utf8(&buf[..n]).unwrap().starts_with(r#"{"Hello""#));
        let msg = std::ffi::CString::new(r#""GetInventory""#).unwrap();
        assert!(sb_die_phone_send(die, msg.as_ptr()));
        assert!(!sb_die_phone_send(die, std::ptr::null()));
        assert!(!sb_die_phone_send(std::ptr::null_mut(), msg.as_ptr()));
        sb_die_free(die);
    }
}
