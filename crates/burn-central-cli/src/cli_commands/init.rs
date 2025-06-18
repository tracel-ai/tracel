use crate::context::CliContext;
use crate::print_warn;
use clap::Args;

#[derive(Args, Debug)]
pub struct InitArgs {}

pub fn handle_command(args: InitArgs, context: CliContext) -> anyhow::Result<()> {
    print_warn!(
        "The `init` command is not implemented yet. Please use the `burn init` command instead."
    );
    Ok(())
}