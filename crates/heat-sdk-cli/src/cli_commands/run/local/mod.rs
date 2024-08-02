pub mod inference;
pub mod training;

use clap::Parser;

use crate::{
    cli_commands::run::local::{inference::LocalInferenceRunArgs, training::LocalTrainingRunArgs},
    context::HeatCliContext,
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

pub(crate) fn handle_command(
    args: LocalRunSubcommand,
    context: HeatCliContext,
) -> anyhow::Result<()> {
    match args {
        LocalRunSubcommand::Training(training_args) => {
            training::handle_command(training_args, context)
        }
        LocalRunSubcommand::Inference(inference_args) => inference::handle_command(inference_args),
    }
}
