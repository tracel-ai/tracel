pub mod inference;
pub mod training;

use crate::context::HeatCliCrateContext;
use clap::{Parser, Subcommand};
use inference::InferenceRunArgs;
use training::TrainingRunArgs;

/// Run a training or inference locally or trigger a remote run.
/// Only local training is supported at the moment.
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// {training|inference} : Run a training or inference locally.
    #[clap(subcommand)]
    pub command: RunCommandType,
}

#[derive(Subcommand, Debug)]
pub enum RunCommandType {
    Training(TrainingRunArgs),
    Inference(InferenceRunArgs),
}
pub(crate) fn handle_command(args: RunArgs, context: &mut HeatCliCrateContext) -> anyhow::Result<()> {
    match args.command {
        RunCommandType::Training(training_args) => {
            training::handle_command(training_args, context)
        }
        RunCommandType::Inference(inference_args) => {
            inference::handle_command(inference_args, context)
        }
    }
}