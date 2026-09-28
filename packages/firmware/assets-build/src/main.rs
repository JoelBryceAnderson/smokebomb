//! Write the standard asset pack to a file, for flashing to the QSPI flash.
//!
//! `cargo run -p smokebomb-assets-build -- smokebomb.smkb`

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "smokebomb.smkb".into());
    let pack = smokebomb_assets_build::standard_pack();
    std::fs::write(&path, &pack).unwrap_or_else(|e| panic!("writing {path}: {e}"));
    println!("wrote {path} ({:.1} MB)", pack.len() as f64 / 1_048_576.0);
}
