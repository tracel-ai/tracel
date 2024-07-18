use clap::Parser;

use super::{
    local::{self, LocalRunSubcommand},
    remote::{self, RemoteRunSubcommand},
};

/// Run a training or inference locally or trigger a remote run.
/// Only local training is supported at the moment.
#[derive(Parser, Debug)]
pub enum RunLocationType {
    /// {training|inference} : Run a training or inference locally.
    #[command(subcommand)]
    Local(LocalRunSubcommand),
    /// todo
    #[command(subcommand)]
    Remote(RemoteRunSubcommand),
}

pub(crate) fn handle_command(args: RunLocationType) -> anyhow::Result<()> {
    match args {
        RunLocationType::Local(local_args) => local::handle_command(local_args),
        RunLocationType::Remote(remote_args) => remote::handle_command(remote_args),
    }
}
