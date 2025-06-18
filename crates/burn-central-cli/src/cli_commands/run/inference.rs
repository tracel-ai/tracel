use clap::Parser;

use crate::context::CliContext;

/// Run an inference locally.
/// Not yet supported.
#[derive(Parser, Debug)]
pub struct InferenceRunArgs {}

pub(crate) fn handle_command(
    _args: InferenceRunArgs,
    _context: CliContext,
) -> anyhow::Result<()> {
    todo!("Local inference is not yet supported")
}
