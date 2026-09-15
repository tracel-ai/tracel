//! Every other way the SDK is built: feature corners, the browser, and the vocabulary without
//! `std`. What the ordinary `check` and `test` commands cover on the host is not repeated here.

use tracel_xtask::prelude::*;
use tracel_xtask::utils::process::run_process;

const WASM: &str = "wasm32-unknown-unknown";
const EMBEDDED: &str = "thumbv7m-none-eabi";

/// Crates that build for the browser: every SDK crate below the app layer.
const WASM_CRATES: &[&str] = &[
    "tracel-task",
    "tracel-artifact",
    "tracel-models",
    "tracel-datasets",
    "tracel-experiment",
    "tracel-experiment-remote",
    "tracel-inference",
    "tracel-console",
    "tracel-core",
    "tracel",
];

pub fn handle_command() -> anyhow::Result<()> {
    group!("Feature corners");
    clippy(&["-p", "tracel-task", "--no-default-features"])?;
    clippy(&["-p", "tracel-task", "--features", "tokio"])?;
    clippy(&[
        "-p",
        "tracel-artifact",
        "-p",
        "tracel-models",
        "--no-default-features",
    ])?;
    endgroup!();

    group!("Browser: {WASM}");
    let mut args = Vec::new();
    for krate in WASM_CRATES {
        args.extend(["-p", krate]);
    }
    args.extend(["--target", WASM]);
    clippy(&args)?;
    clippy(&[
        "-p",
        "tracel-artifact",
        "-p",
        "tracel-models",
        "--no-default-features",
        "--target",
        WASM,
    ])?;
    endgroup!();

    group!("Embedded, no_std: {EMBEDDED}");
    clippy(&[
        "-p",
        "tracel-task",
        "--no-default-features",
        "--target",
        EMBEDDED,
    ])?;
    endgroup!();

    Ok(())
}

fn clippy(args: &[&str]) -> anyhow::Result<()> {
    let mut full = vec!["clippy"];
    full.extend_from_slice(args);
    full.extend(["--", "-D", "warnings"]);
    run_process("cargo", &full, None, None, "clippy failed")
}
