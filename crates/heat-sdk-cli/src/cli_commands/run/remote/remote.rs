use clap::Parser;

use crate::cli_commands::remote::{
    inference::{self, RemoteInferenceRunArgs},
    training::{self, RemoteTrainingRunArgs},
};

/// Run a training or inference remotely.
/// Not yet supported.
#[derive(Parser, Debug)]
pub enum RemoteRunSubcommand {
    /// todo
    Training(RemoteTrainingRunArgs),
    /// todo
    Inference(RemoteInferenceRunArgs),
}

pub(crate) fn handle_command(args: RemoteRunSubcommand) -> anyhow::Result<()> {
    match args {
        RemoteRunSubcommand::Training(training_args) => training::handle_command(training_args),
        RemoteRunSubcommand::Inference(inference_args) => inference::handle_command(inference_args),
    }
}
