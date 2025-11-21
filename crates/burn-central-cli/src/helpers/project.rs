//! Project helpers for CLI operations
//!
//! These helpers handle common project checks and provide user-friendly error messages.

use crate::context::CliContext;
use burn_central_lib::{ProjectContext, entity::projects::CrateInfo, tools::cargo};

/// Check if current directory contains a Rust project (has Cargo.toml)
pub fn is_rust_project() -> bool {
    cargo::try_locate_manifest().is_some()
}

/// Check if current directory has a linked Burn Central project
pub fn is_burn_central_project_linked(context: &CliContext) -> bool {
    ProjectContext::load(context.get_burn_dir_name()).is_ok()
}

/// Require a linked Burn Central project, showing helpful errors if not found
pub fn require_linked_project(context: &CliContext) -> anyhow::Result<ProjectContext> {
    match ProjectContext::load(context.get_burn_dir_name()) {
        Ok(project) => Ok(project),
        Err(_) => {
            if is_rust_project() {
                context
                    .terminal()
                    .print_err("This Rust project is not linked to Burn Central.");
                context
                    .terminal()
                    .print("💡 Run 'burn init' to initialize a Burn Central project.");
            } else {
                context
                    .terminal()
                    .print_err("No Rust project found in current directory.");
                context
                    .terminal()
                    .print("💡 Navigate to a Rust project directory first.");
            }
            anyhow::bail!("No linked Burn Central project found")
        }
    }
}

/// Require a Rust project (with or without Burn Central linkage)
pub fn require_rust_project(context: &CliContext) -> anyhow::Result<CrateInfo> {
    match ProjectContext::load_crate_info() {
        Ok(crate_info) => Ok(crate_info),
        Err(_) => {
            context
                .terminal()
                .print_err("No Rust project found in current directory.");
            context
                .terminal()
                .print("💡 Navigate to a directory containing a Cargo.toml file.");
            anyhow::bail!("No Rust project found")
        }
    }
}

/// Check if we're in a valid state for initialization
pub fn can_initialize_project(context: &CliContext, force: bool) -> anyhow::Result<bool> {
    // Must be in a Rust project
    if !is_rust_project() {
        context
            .terminal()
            .print_err("No Rust project found in current directory.");
        context
            .terminal()
            .print("💡 Run this command from a Rust project directory with a Cargo.toml file.");
        return Ok(false);
    }

    // Check if already linked
    if is_burn_central_project_linked(context) {
        if force {
            context
                .terminal()
                .print("🔄 Force flag provided - reinitializing existing project.");
            return Ok(true);
        } else {
            context
                .terminal()
                .print("✅ Project is already linked to Burn Central.");
            context
                .terminal()
                .print("💡 Use --force flag to reinitialize.");
            return Ok(false);
        }
    }

    // All good - can initialize
    Ok(true)
}
