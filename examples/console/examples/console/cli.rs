//! Command-line surface for the console example, grouped by noun the way `git`/`gh` are:
//! `console <noun> <verb>`.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use tracel::console::Env;

/// The owner namespace and project name shared by every command scoped to one project.
#[derive(Debug, Args)]
pub struct ProjectScope {
    /// Owner namespace. Prompts interactively when omitted.
    pub namespace: Option<String>,
    /// Project name. Prompts interactively when omitted.
    pub project: Option<String>,
}

/// Explore the Tracel console API from a terminal.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// Console environment: production, development, or staging:N.
    #[arg(
        long,
        short,
        global = true,
        env = "TRACEL_ENV",
        default_value = "development",
        value_parser = parse_env
    )]
    pub environment: Env,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Sign in, sign out, and inspect the current session.
    #[command(subcommand)]
    Auth(AuthCommand),
    /// Browse organizations available to the current session.
    #[command(subcommand)]
    Org(OrgCommand),
    /// Browse projects.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Browse and download models.
    #[command(subcommand)]
    Model(ModelCommand),
}

#[derive(Debug, Subcommand)]
pub enum AuthCommand {
    /// Sign in through the OAuth device flow.
    Login,
    /// Clear the session saved for this environment.
    Logout,
    /// Display the signed-in user.
    Whoami,
}

#[derive(Debug, Subcommand)]
pub enum OrgCommand {
    /// List organizations available to the current session.
    List,
}

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List the projects in a namespace.
    List {
        /// User or organization namespace. Prompts interactively when omitted.
        namespace: Option<String>,
    },
    /// Show one project's details.
    Show {
        #[command(flatten)]
        scope: ProjectScope,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// List the models and versions in a project.
    List {
        #[command(flatten)]
        scope: ProjectScope,
    },
    /// Show one model's details and published versions.
    Show {
        #[command(flatten)]
        scope: ProjectScope,
        /// Model name. Prompts interactively when omitted.
        #[arg(env = "TRACEL_MODEL")]
        model: Option<String>,
    },
    /// Download a model version to a local directory.
    Download {
        #[command(flatten)]
        scope: ProjectScope,
        /// Model name. Prompts interactively when omitted.
        #[arg(env = "TRACEL_MODEL")]
        model: Option<String>,
        /// Version number to download. Prompts interactively when omitted.
        version: Option<u32>,
        /// Directory to write the version into. Defaults to `<model>-v<version>`.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

fn parse_env(value: &str) -> Result<Env, String> {
    match value {
        "production" => Ok(Env::Production),
        "development" => Ok(Env::Development),
        other => match other.strip_prefix("staging:").map(str::parse::<u8>) {
            Some(Ok(version)) => Ok(Env::Staging(version)),
            _ => Err(format!(
                "unknown environment {other:?}; expected `production`, `development`, or `staging:N`"
            )),
        },
    }
}
