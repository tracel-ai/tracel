use std::path::PathBuf;

use clap::Parser;

use crate::crate_gen::backend::BackendType;

/// Run an inference locally.
/// Not yet supported.
#[derive(Parser, Debug)]
pub struct LocalInferenceRunArgs {
    function: String,
    model_path: PathBuf,
    /// Backend to use
    #[clap(short = 'b', long = "backends", value_delimiter = ' ', num_args = 1.., required = true)]
    backends: Vec<BackendType>,
    /// The project ID
    // todo: support project name and creating a project if it doesn't exist
    #[clap(short = 'p', long = "project", required = true)]
    project: String,
    /// The API key
    #[clap(short = 'k', long = "key", required = true)]
    key: String,
}

pub(crate) fn handle_command(_args: LocalInferenceRunArgs) -> anyhow::Result<()> {
    todo!("Local inference is not yet supported")
}
