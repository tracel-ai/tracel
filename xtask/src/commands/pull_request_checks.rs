use super::ci::{self, CICmdArgs};

use anyhow::Ok;

pub(crate) fn handle_command() -> anyhow::Result<()> {
    ci::handle_command(CICmdArgs {
        target: super::Target::All,
        command: ci::CICommand::All
    })?;
    Ok(())
}
