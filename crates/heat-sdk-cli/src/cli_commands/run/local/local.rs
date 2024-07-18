use clap::Parser;

use crate::cli_commands::local::{
    inference::{self, LocalInferenceRunArgs},
    training::{self, LocalTrainingRunArgs},
};

/// Run a training or inference locally.
/// Only local training is supported at the moment.
#[derive(Parser, Debug)]
pub enum LocalRunSubcommand {
    /// Run a training locally.
    Training(LocalTrainingRunArgs),
    /// Run an inference locally.
    Inference(LocalInferenceRunArgs),
}

pub(crate) fn handle_command(args: LocalRunSubcommand) -> anyhow::Result<()> {
    match args {
        LocalRunSubcommand::Training(training_args) => training::handle_command(training_args),
        LocalRunSubcommand::Inference(inference_args) => inference::handle_command(inference_args),
    }
}
