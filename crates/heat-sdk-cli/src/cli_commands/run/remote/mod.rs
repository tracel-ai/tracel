pub mod inference;
pub mod training;

use clap::Parser;

use crate::{
    cli_commands::run::remote::{
        inference::RemoteInferenceRunArgs, training::RemoteTrainingRunArgs,
    },
    context::HeatCliContext,
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

pub(crate) fn handle_command(
    args: RemoteRunSubcommand,
    context: HeatCliContext,
) -> anyhow::Result<()> {
    match args {
        RemoteRunSubcommand::Training(training_args) => {
            training::handle_command(training_args, context)
        }
        RemoteRunSubcommand::Inference(inference_args) => inference::handle_command(inference_args),
    }
}
