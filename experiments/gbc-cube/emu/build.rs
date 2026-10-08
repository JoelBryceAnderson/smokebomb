//! Compiles walnut-cgb (vendored, single header) behind `csrc/shim.c`.

fn main() {
    println!("cargo:rerun-if-changed=csrc/shim.c");
    println!("cargo:rerun-if-changed=csrc/shim.h");
    println!("cargo:rerun-if-changed=vendor/walnut-cgb/walnut_cgb.h");
    cc::Build::new()
        .file("csrc/shim.c")
        .include("csrc")
        .include("vendor/walnut-cgb")
        // The core is the hot loop of the whole simulator: always optimise
        // it, even in debug builds.
        .opt_level(2)
        .flag_if_supported("-fno-strict-aliasing")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-variable")
        .flag_if_supported("-Wno-unused-but-set-variable")
        .flag_if_supported("-Wno-unused-function")
        .flag_if_supported("-Wno-sign-compare")
        .flag_if_supported("-Wno-missing-field-initializers")
        .flag_if_supported("-Wno-implicit-fallthrough")
        .flag_if_supported("-Wno-type-limits")
        .compile("walnut_cgb");
}
