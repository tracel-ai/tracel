use clap::Parser;

/// Run a training remotely.
/// Not yet supported.
#[derive(Parser, Debug)]
pub struct RemoteTrainingRunArgs {
    //todo
}

pub(crate) fn handle_command(_args: RemoteTrainingRunArgs) -> anyhow::Result<()> {
    todo!("Remote training is not yet supported")
}
