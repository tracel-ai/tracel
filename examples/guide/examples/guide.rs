//
// Note: If you are following the Burn Book guide this file can be ignored.
//
// This example file is added only for convenience and consistency so that
// the guide example can be executed like any other examples using:
//
//     cargo run --release --example guide
//
use std::{env, process::Command};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    Command::new("cargo")
        .args(["run", "--bin", "guide", "--"])
        .args(&args)
        .status()
        .expect("guide example should run");
}
