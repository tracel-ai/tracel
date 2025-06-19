use crate::context::CliContext;
use crate::print_warn;
use anyhow::Context;
use clap::Args;

#[derive(Args, Debug)]
pub struct InitArgs {}

pub fn handle_command(args: InitArgs, context: CliContext) -> anyhow::Result<()> {
    print_warn!(
        "The `init` command is not implemented yet."
    );
    Ok(())
}