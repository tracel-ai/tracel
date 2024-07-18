use clap::Parser;

/// Run an inference remotely.
/// Not yet supported.
#[derive(Parser, Debug)]
pub struct RemoteInferenceRunArgs {
    //todo
}

pub(crate) fn handle_command(_args: RemoteInferenceRunArgs) -> anyhow::Result<()> {
    todo!("Remote inference is not yet supported")
}
