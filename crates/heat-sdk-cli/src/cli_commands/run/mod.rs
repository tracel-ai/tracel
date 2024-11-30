pub mod inference;
pub mod training;

use clap::Parser;
use inference::InferenceRunArgs;
use training::TrainingRunArgs;

use crate::context::HeatCliContext;

/// Run a training or inference locally or trigger a remote run.
/// Only local training is supported at the moment.
#[derive(Parser, Debug)]
pub enum RunLocationType {
    /// {training|inference} : Run a training or inference locally.
    Training(TrainingRunArgs),
    /// todo
    Inference(InferenceRunArgs),
}

pub(crate) fn handle_command(args: RunLocationType, context: HeatCliContext) -> anyhow::Result<()> {
    match args {
        RunLocationType::Training(training_args) => {
            training::handle_command(training_args, context)
        }
        RunLocationType::Inference(inference_args) => {
            inference::handle_command(inference_args, context)
        }
    }
}
