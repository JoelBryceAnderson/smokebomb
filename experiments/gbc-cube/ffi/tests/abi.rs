//! The C ABI as the app calls it: open, tick, read faces, errors, saves.

use std::ffi::{c_char, CString};

use gbc_cube_ffi::*;

fn text(f: impl Fn(*mut c_char, usize) -> usize) -> String {
    let mut buf = vec![0 as c_char; 128];
    let n = f(buf.as_mut_ptr(), buf.len());
    let bytes: Vec<u8> = buf[..n.min(127)].iter().map(|&c| c as u8).collect();
    String::from_utf8(bytes).unwrap()
}

#[test]
fn the_demo_cart_runs_and_draws_the_map() {
    unsafe {
        let cube = gc_cube_open(std::ptr::null());
        assert!(!cube.is_null());
        assert_eq!(gc_cube_panel_side(cube), 64);
        assert_eq!(text(|o, l| gc_cube_title(cube, o, l)), "GBCCUBEDEMO");
        let level = GcImu {
            accel_mg: [0, 1000, 0],
            gyro_mdps: [0; 3],
        };
        let mut seq = 0;
        for i in 0..120 {
            // Walk right for a second, on the joypad.
            let keys = if i > 30 { 0x10 } else { 0 };
            let s = gc_cube_tick(cube, level, 0, keys);
            assert_ne!(s, seq, "the faces change every tick");
            seq = s;
        }
        let status = text(|o, l| gc_cube_status(cube, o, l));
        assert!(status.starts_with("Overworld, world from RAM, up +Y"), "{status}");
        // The up face (+Y) has the map on it: not all one colour.
        let mut face = vec![0u8; 64 * 64 * 4];
        assert!(gc_cube_face_rgba(cube, 2, face.as_mut_ptr(), face.len()));
        let first = &face[..4];
        assert!(face.chunks(4).any(|p| p != first));
        assert!(face.chunks(4).all(|p| p[3] == 255));
        // Too small a buffer, or no such face.
        assert!(!gc_cube_face_rgba(cube, 2, face.as_mut_ptr(), 100));
        assert!(!gc_cube_face_rgba(cube, 6, face.as_mut_ptr(), face.len()));
        gc_cube_set_ui(cube, GC_UI_PAN);
        gc_cube_set_world(cube, false);
        gc_cube_tick(cube, level, 0, 0);
        assert!(text(|o, l| gc_cube_status(cube, o, l)).contains("frame folded"));
        gc_cube_free(cube);
    }
}

#[test]
fn a_bad_rom_says_why() {
    unsafe {
        let path = CString::new("/nonexistent/game.gbc").unwrap();
        assert!(gc_cube_open(path.as_ptr()).is_null());
        let why = text(|o, l| gc_cube_last_error(o, l));
        assert!(why.contains("game.gbc"), "{why}");

        let dir = std::env::temp_dir().join(format!("gbc-cube-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.gbc");
        std::fs::write(&junk, vec![0u8; 32 * 1024]).unwrap();
        let path = CString::new(junk.to_str().unwrap()).unwrap();
        assert!(gc_cube_open(path.as_ptr()).is_null());
        assert!(text(|o, l| gc_cube_last_error(o, l)).contains("checksum"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn the_die_firmware_abi_is_carried_too() {
    // The app links this library instead of libsmokebomb_ffi.a.
    let die = gbc_cube_ffi::smokebomb_ffi::sb_die_new(1);
    assert!(!die.is_null());
    unsafe { gbc_cube_ffi::smokebomb_ffi::sb_die_free(die) };
}
