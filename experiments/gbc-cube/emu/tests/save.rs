//! The battery save round-trips through the `.sav` file, clock included.

use gbc_cube_emu::{rtc, Emulator};

/// A 32 KiB cartridge with MBC5, 8 KiB of battery RAM and a `jr` loop.
fn rom() -> Vec<u8> {
    let mut rom = vec![0u8; 32 * 1024];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]);
    rom[0x150..0x152].copy_from_slice(&[0x18, 0xFE]);
    rom[0x134..0x138].copy_from_slice(b"SAVE");
    rom[0x143] = 0x80;
    rom[0x147] = 0x1B;
    rom[0x149] = 0x02;
    let mut x = 0u8;
    for b in &rom[0x134..=0x14C] {
        x = x.wrapping_sub(*b).wrapping_sub(1);
    }
    rom[0x14D] = x;
    rom
}

#[test]
fn cart_ram_survives_a_restart() {
    let dir = std::env::temp_dir().join(format!("gbc-cube-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("game.sav");
    let _ = std::fs::remove_file(&path);

    let mut emu = Emulator::new(rom()).unwrap();
    emu.attach_save(path.clone()).unwrap();
    assert_eq!(emu.cart_ram().len(), 8192);
    emu.run_frame().unwrap();
    // Enable cart RAM through the MBC and write two bytes.
    emu.write(0x0000, 0x0A);
    emu.write(0xA000, 0x42);
    emu.write(0xBFFF, 0x99);
    assert!(emu.flush_save(true).unwrap());
    assert!(!emu.flush_save(true).unwrap(), "unchanged: no second write");
    drop(emu);

    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), 8192 + rtc::FOOTER_LEN);
    assert!(rtc::Footer::parse(&bytes[8192..]).is_some());

    let mut emu = Emulator::new(rom()).unwrap();
    emu.attach_save(path.clone()).unwrap();
    assert_eq!(emu.cart_ram()[0], 0x42);
    assert_eq!(emu.cart_ram()[8191], 0x99);
    emu.write(0x0000, 0x0A);
    assert_eq!(emu.read(0xA000), 0x42);
    std::fs::remove_dir_all(&dir).unwrap();
}
