//! This script is run before a PR is created.
//!
//! It is used to check that the code compiles and passes all tests.
//!
//! It is also used to check that the code is formatted correctly and passes clippy.

use std::collections::HashMap;
use std::env;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::logging::init_logger;
use crate::utils::cargo::{run_cargo, run_cargo_with_path};
use crate::utils::process::{handle_child_process, run_command};
use crate::utils::rustup::rustup_add_component;
use crate::utils::time::format_duration;
use crate::utils::workspace::{get_workspace_members, WorkspaceMemberType};
use crate::utils::Params;
use crate::{endgroup, group};

#[derive(clap::ValueEnum, Default, Copy, Clone, PartialEq, Eq)]
pub(crate) enum CheckType {
    /// Run all checks.
    #[default]
    All,
    /// Run `std` environment checks
    Std,
    /// Check for typos
    Typos,
    /// Test the examples
    Examples,
}

impl CheckType {
    pub(crate) fn run(&self) -> anyhow::Result<()> {
        // Setup logger
        init_logger().init();

        // Start time measurement
        let start = Instant::now();

        // The environment can assume ONLY "std", "no_std", "typos", "examples"
        //
        // Depending on the input argument, the respective environment checks
        // are run.
        //
        // If no environment has been passed, run all checks.
        match self {
            Self::Std => std_checks(),
            Self::Typos => check_typos(),
            Self::Examples => check_examples(),
            Self::All => {
                /* Run all checks */
                check_typos();
                std_checks();
                check_examples();
            }
        }

        // Stop time measurement
        //
        // Compute runtime duration
        let duration = start.elapsed();

        // Print duration
        info!(
            "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
            format_duration(&duration)
        );

        Ok(())
    }
}

/// Run cargo build command
fn cargo_build(params: Params) {
    // Run cargo build
    run_cargo(
        "build",
        params + "--color=always",
        HashMap::new(),
        "Failed to run cargo build",
    );
}

/// Run cargo install command
fn cargo_install(params: Params) {
    // Run cargo install
    run_cargo(
        "install",
        params + "--color=always",
        HashMap::new(),
        "Failed to run cargo install",
    );
}

/// Run cargo test command
fn cargo_test(params: Params) {
    // Run cargo test
    run_cargo(
        "test",
        params + "--color=always" + "--" + "--color=always",
        HashMap::new(),
        "Failed to run cargo test",
    );
}

/// Run cargo fmt command
fn cargo_fmt() {
    group!("Cargo: fmt");
    run_cargo(
        "fmt",
        ["--check", "--all", "--", "--color=always"].into(),
        HashMap::new(),
        "Failed to run cargo fmt",
    );
    endgroup!();
}

/// Run cargo clippy command
fn cargo_clippy() {
    if std::env::var("CI").is_ok() {
        return;
    }
    // Run cargo clippy
    run_cargo(
        "clippy",
        ["--color=always", "--all-targets", "--", "-D", "warnings"].into(),
        HashMap::new(),
        "Failed to run cargo clippy",
    );
}

/// Run cargo doc command
fn cargo_doc(params: Params) {
    // Run cargo doc
    run_cargo(
        "doc",
        params + "--color=always",
        HashMap::new(),
        "Failed to run cargo doc",
    );
}

// Setup code coverage
fn setup_coverage() {
    // Install llvm-tools-preview
    rustup_add_component("llvm-tools-preview");

    // Set coverage environment variables
    env::set_var("RUSTFLAGS", "-Cinstrument-coverage");
    env::set_var("LLVM_PROFILE_FILE", "burn-%p-%m.profraw");
}

// Run grcov to produce lcov.info
fn run_grcov() {
    // grcov arguments
    #[rustfmt::skip]
    let args = [
        ".",
        "--binary-path", "./target/debug/",
        "-s", ".",
        "-t", "lcov",
        "--branch",
        "--ignore-not-existing",
        "--ignore", "/*", // It excludes std library code coverage from analysis
        "--ignore", "xtask/*",
        "--ignore", "examples/*",
        "-o", "lcov.info",
    ];

    run_command(
        "grcov",
        &args,
        "Failed to run grcov",
        "Failed to wait for grcov child process",
    );
}

fn std_checks() {
    // Set RUSTDOCFLAGS environment variable to treat warnings as errors
    // for the documentation build
    env::set_var("RUSTDOCFLAGS", "-D warnings");

    // Check if COVERAGE environment variable is set
    let is_coverage = std::env::var("COVERAGE").is_ok();

    // Check format
    cargo_fmt();

    // Check clippy lints
    cargo_clippy();

    // Produce documentation for each workspace member
    group!("Docs: crates");
    let params = Params::from(["--workspace", "--no-deps"]);
    // Exclude burn-cuda on all platforms
    cargo_doc(params);
    endgroup!();

    // Setup code coverage
    if is_coverage {
        setup_coverage();
    }

    // Build & test each member in workspace
    let members = get_workspace_members(WorkspaceMemberType::Crate);
    for member in members {
        group!("Checks: {}", member.name);
        cargo_build(Params::from(["-p", &member.name]));
        cargo_test(Params::from(["-p", &member.name]));
        endgroup!();
    }

    // Run grcov and produce lcov.info
    if is_coverage {
        run_grcov();
    }
}

fn check_typos() {
    // This path defines where typos-cli is installed on different
    // operating systems.
    let typos_cli_path = std::env::var("CARGO_HOME")
        .map(|v| std::path::Path::new(&v).join("bin/typos-cli"))
        .unwrap();

    // Do not run cargo install on CI to speed up the computation.
    // Check whether the file has been installed on
    if std::env::var("CI").is_err() && !typos_cli_path.exists() {
        // Install typos-cli
        cargo_install(["typos-cli", "--version", "1.16.5"].into());
    }

    info!("Running typos check \n\n");

    // Run typos command as child process
    let typos = Command::new("typos")
        .stdout(Stdio::inherit()) // Send stdout directly to terminal
        .stderr(Stdio::inherit()) // Send stderr directly to terminal
        .spawn()
        .expect("Failed to run typos");

    // Handle typos child process
    handle_child_process(typos, "Failed to wait for typos child process");
}

fn check_examples() {
    let members = get_workspace_members(WorkspaceMemberType::Example);
    for member in members {
        if member.name == "notebook" {
            continue;
        }

        group!("Checks: Example - {}", member.name);
        run_cargo_with_path(
            "check",
            ["--examples"].into(),
            HashMap::new(),
            Some(member.path),
            "Failed to check example",
        );
        endgroup!();
    }
}
