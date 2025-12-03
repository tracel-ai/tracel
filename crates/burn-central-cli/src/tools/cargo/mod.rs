//! All of the code in this directory is heavily inspired by the [Cargo source code](https://github.com/rust-lang/cargo)
//! and modified to fit the needs of the burn-central-cli project.
//!
//! In files that are entirely copied from Cargo, a link to the original source code is included in the top of the file as a comment.
//! In files that are partially copied from Cargo or include functions/definitons from multiple different files in Cargo, a link to the source code of the original functions/definitions is included in the comments above the copied code.
//!
//! Definitions and functions that are not copied from Cargo do not have a link to the original source code.

mod dependency;
mod features;
mod interning;
pub mod package;
mod paths;
mod restricted_names;
pub mod schemas;
mod toml;
mod version;
mod workspace;

use std::ffi::OsString;

pub fn cargo_binary() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

pub fn try_locate_manifest() -> Option<std::path::PathBuf> {
    let output = command()
        .arg("locate-project")
        .output()
        .expect("Failed to run cargo locate-project");
    if !output.status.success() {
        return None;
    }

    let output_str = String::from_utf8(output.stdout).expect("Failed to parse output");
    if output_str.trim().is_empty() {
        return None;
    }
    let parsed_output: serde_json::Value =
        serde_json::from_str(&output_str).expect("Failed to parse output");

    let manifest_path_str = parsed_output["root"]
        .as_str()
        .expect("Failed to parse output")
        .to_owned();

    let manifest_path = std::path::PathBuf::from(manifest_path_str);
    Some(manifest_path)
}

pub fn command() -> std::process::Command {
    std::process::Command::new(cargo_binary())
}
