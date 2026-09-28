//! Build the asset pack the simulated QSPI flash starts with: Space Grotesk
//! at every size plus the placeholder smoke clips.

fn main() {
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("pack.smkb"), smokebomb_assets_build::standard_pack()).expect("write pack");
    println!("cargo:rerun-if-changed=build.rs");
}
