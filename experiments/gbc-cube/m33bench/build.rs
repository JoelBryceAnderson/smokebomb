//! Compile walnut-cgb for the Cortex-M33 and embed the workloads:
//! `GBC_CUBE_M33_ROM` (a ROM for the emulator benchmark) and
//! `GBC_CUBE_M33_SNAPSHOT` (a `gbc-cube snapshot` directory for the
//! renderer benchmark), and optionally `GBC_CUBE_M33_SAV` (the ROM's battery
//! save, so the script can CONTINUE into the overworld). Any can be left
//! out.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::copy("memory.x", out.join("memory.x")).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rerun-if-changed=../emu/csrc/shim.c");
    println!("cargo:rerun-if-env-changed=GBC_CUBE_M33_ROM");
    println!("cargo:rerun-if-env-changed=GBC_CUBE_M33_SNAPSHOT");

    cc::Build::new()
        .file("../emu/csrc/shim.c")
        .include("include")
        .include("../emu/csrc")
        .include("../emu/vendor/walnut-cgb")
        .define("GCB_NO_SETJMP", None)
        .flag("-ffreestanding")
        .flag("-mcpu=cortex-m33")
        .flag("-mfloat-abi=hard")
        .flag("-mfpu=fpv5-sp-d16")
        .flag_if_supported("-Wno-everything")
        .opt_level(2)
        .compile("walnut_cgb");

    let embed = |var: &str, file: &str| match env::var(var) {
        Ok(p) => format!(
            "include_bytes!({:?})",
            PathBuf::from(p).join(file).display().to_string()
        ),
        Err(_) => "&[]".into(),
    };
    let rom = match env::var("GBC_CUBE_M33_ROM") {
        Ok(p) => format!("include_bytes!({p:?})"),
        Err(_) => "&[]".into(),
    };
    let snap = |part: &str, f: &str| embed("GBC_CUBE_M33_SNAPSHOT", &format!("{part}/{f}"));
    let sav = match env::var("GBC_CUBE_M33_SAV") {
        Ok(p) => format!("include_bytes!({p:?})"),
        Err(_) => "&[]".into(),
    };
    println!("cargo:rerun-if-env-changed=GBC_CUBE_M33_SAV");
    println!("cargo:rerun-if-env-changed=GBC_CUBE_M33_WARMUP");
    let mut src = format!("pub static CPU_ROM: &[u8] = {rom};\npub static CPU_SAV: &[u8] = {sav};\n");
    src += &format!(
        "pub static SNAP_ROM: &[u8] = {};\n",
        embed("GBC_CUBE_M33_SNAPSHOT", "rom.bin")
    );
    for part in ["walk", "text"] {
        src += &format!("pub mod {part} {{\n");
        for (name, f) in [
            ("WRAM", "wram.bin"),
            ("VRAM", "vram.bin"),
            ("OAM", "oam.bin"),
            ("IO", "io.bin"),
            ("BGPAL", "bgpal.bin"),
            ("OBJPAL", "objpal.bin"),
            ("FRAME", "frame.bin"),
        ] {
            src += &format!("    pub static {name}: &[u8] = {};\n", snap(part, f));
        }
        src += "}\n";
    }
    fs::write(out.join("data.rs"), src).unwrap();
}
