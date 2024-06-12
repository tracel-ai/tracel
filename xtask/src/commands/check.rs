use std::process::Command;

use anyhow::{Ok, Result, anyhow};
use clap::{Args, Subcommand};
use strum::{Display, EnumIter, EnumString, IntoEnumIterator};

use crate::utils::{prompt::{ask_once, separator}, workspace::{get_workspace_members, WorkspaceMemberType}};

use super::Target;

#[derive(Args)]
pub(crate) struct CheckCmdArgs {
    /// Target to check for.
    #[arg(short, long, value_enum)]
    target: Target,
    #[command(subcommand)]
    command: CheckCommand,
}

#[derive(EnumString, EnumIter, Display, Clone, PartialEq, Subcommand)]
#[strum(serialize_all = "lowercase")]
enum CheckCommand {
    /// Run format command.
    Format,
    /// Run ling command.
    Lint,
    /// Run all the checks.
    All,
}

pub(crate) fn handle_command(args: CheckCmdArgs, answer: Option<bool>) -> anyhow::Result<()> {
    match args.command {
        CheckCommand::Format => run_format(&args.target, answer),
        CheckCommand::Lint => run_lint(&args.target, answer),
        CheckCommand::All => {
            let answer = ask_once("This will run all the checks with autofix on all members of the workspace.");
            CheckCommand::iter()
                .filter(|c| *c != CheckCommand::All)
                .try_for_each(|c| handle_command(
                    CheckCmdArgs {
                        command: c,
                        target: args.target.clone()
                    },
                    Some(answer),
                ))
        },
    }
}

fn run_format(target: &Target, mut answer: Option<bool>) -> Result<()> {
    match target {
        Target::Crates | Target::Examples => {
            let members = match target {
                Target::Crates => get_workspace_members(WorkspaceMemberType::Crate),
                Target::Examples => get_workspace_members(WorkspaceMemberType::Example),
                _ => unreachable!(),
            };

            if answer.is_none() {
                answer = Some(ask_once(&format!(
                    "This will run format checks on all {} of the workspace.",
                    if *target == Target::Crates { "crates" } else { "examples" }
                )));
            }

            if answer.unwrap() {
                for member in members {
                    separator();
                    info!("Running cargo fmt on member: {}", &member.name);
                    info!("Command line: cargo fmt -p {}", &member.name);
                    let status = Command::new("cargo")
                        .args(["fmt", "-p", &member.name])
                        .status()
                        .map_err(|e| anyhow!("Failed to execute cargo fmt: {}", e))?;
                    if !status.success() {
                        return Err(anyhow!("Format check execution failed for {}", &member.name));
                    }
                }
            }
        },
        Target::All => {
            if answer.is_none() {
                answer = Some(ask_once("This will run format check on all members of the workspace."));
            }
            if answer.unwrap() {
                Target::iter()
                    .filter(|t| *t != Target::All)
                    .try_for_each(|t| run_format(&t, answer))?;
            }
        },
    }
    Ok(())
}

fn run_lint(target: &Target, mut answer: Option<bool>) -> anyhow::Result<()> {
    match target {
        Target::Crates | Target::Examples => {
            let members = match target {
                Target::Crates => get_workspace_members(WorkspaceMemberType::Crate),
                Target::Examples => get_workspace_members(WorkspaceMemberType::Example),
                _ => unreachable!(),
            };

            if answer.is_none() {
                answer = Some(ask_once(&format!(
                    "This will run lint fix on all {} of the workspace.",
                    if *target == Target::Crates { "crates" } else { "examples" }
                )));
            }

            if answer.unwrap() {
                for member in members {
                    separator();
                    info!("Running cargo clippy on member: {}", &member.name);
                    info!("Command line: cargo clippy --no-deps --fix --allow-dirty -p {}", &member.name);
                    let status = Command::new("cargo")
                        .args(["clippy", "--no-deps", "--fix", "--allow-dirty", "-p", &member.name])
                        .status()
                        .map_err(|e| anyhow!("Failed to execute cargo clippy: {}", e))?;
                    if !status.success() {
                        return Err(anyhow!("Lint fix execution failed for {}", &member.name));
                    }
                }
            }
        },
        Target::All => {
            if answer.is_none() {
                answer = Some(ask_once("This will run lint fix on all members of the workspace."));
            }
            if answer.unwrap() {
                Target::iter()
                    .filter(|t| *t != Target::All)
                    .try_for_each(|t| run_lint(&t, answer))?;
            }
        },
    }
    Ok(())
}
