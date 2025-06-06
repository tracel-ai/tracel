use crate::print_info;
use std::ffi::OsString;

pub fn cargo_binary() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

pub fn try_locate_manifest() -> Option<std::path::PathBuf> {
    let output = command()
        .arg("locate-project")
        .output()
        .expect("Failed to run cargo locate-project");
    let output_str = String::from_utf8(output.stdout).expect("Failed to parse output");
    let parsed_output: serde_json::Value = serde_json::from_str(&output_str).expect("Failed to parse output");

    let manifest_path_str = parsed_output["root"]
        .as_str()
        .expect("Failed to parse output")
        .to_owned();

    let manifest_path = std::path::PathBuf::from(manifest_path_str);
    print_info!("Found manifest at: {}", manifest_path.display());
    Some(manifest_path)
}

pub fn command() -> std::process::Command {
    let mut cmd = std::process::Command::new(cargo_binary());
    cmd
}
