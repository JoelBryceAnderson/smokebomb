//! `gbc-cube`: Pokémon Crystal (or the demo cart) on the simulated cube.
//!
//! ```text
//! gbc-cube serve  [--rom PATH | --demo] [--port 3100]
//! gbc-cube bench  [--rom PATH | --demo] [--frames 3600] [--json OUT]
//! gbc-cube shots  [--rom PATH | --demo] [--out DIR]
//! gbc-cube snapshot --out DIR     (demo RAM for the Cortex-M33 benchmark)
//! ```
//!
//! The ROM comes from `--rom` or the `GBC_CUBE_ROM` environment variable;
//! without either, the built-in demo cart runs. A ROM's battery save is read
//! from and written to the same path with a `.sav` extension.

mod bench;
mod image;
mod server;
mod session;
mod shots;
mod snapshot;

use std::path::PathBuf;

use anyhow::{bail, Result};
use session::Cart;

struct Args {
    cmd: String,
    rom: Option<PathBuf>,
    demo: bool,
    port: u16,
    frames: u64,
    json: Option<PathBuf>,
    out: PathBuf,
}

fn parse() -> Result<Args> {
    let mut a = Args {
        cmd: "serve".into(),
        rom: None,
        demo: false,
        port: 3100,
        frames: 3600,
        json: None,
        out: PathBuf::from("shots"),
    };
    let mut it = std::env::args().skip(1);
    let mut first = true;
    while let Some(arg) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| anyhow::anyhow!("{name} needs a value"));
        match arg.as_str() {
            "--rom" => a.rom = Some(val("--rom")?.into()),
            "--demo" => a.demo = true,
            "--port" => a.port = val("--port")?.parse()?,
            "--frames" => a.frames = val("--frames")?.parse()?,
            "--json" => a.json = Some(val("--json")?.into()),
            "--out" => a.out = val("--out")?.into(),
            "-h" | "--help" => {
                println!("{}", include_str!("usage.txt"));
                std::process::exit(0);
            }
            c if first && !c.starts_with('-') => a.cmd = c.to_string(),
            other => bail!("unknown argument {other:?} (try --help)"),
        }
        first = false;
    }
    Ok(a)
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "gbc_cube=info".into()),
        )
        .init();
    let a = parse()?;
    let cart = Cart::from_args(a.rom.clone(), a.demo);
    if let Cart::Demo = cart {
        if a.rom.is_none() && !a.demo {
            tracing::info!("no ROM (--rom or GBC_CUBE_ROM): running the demo cart");
        }
    }
    match a.cmd.as_str() {
        "serve" => {
            let web = std::env::var_os("GBC_CUBE_WEB_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/dist")));
            tokio::runtime::Runtime::new()?.block_on(server::serve(cart, a.port, web))
        }
        "bench" => bench::run(&cart, a.frames, a.json.as_deref()),
        "shots" => shots::run(&cart, &a.out),
        "snapshot" => snapshot::run(&a.out),
        c => bail!("unknown command {c:?}: serve, bench, shots or snapshot"),
    }
}
