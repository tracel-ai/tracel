use crate::context::CliContext;
use crate::print_warn;
use clap::Args;

#[derive(Args, Debug)]
pub struct InitArgs {}

pub fn handle_command(_args: InitArgs, _context: CliContext) -> anyhow::Result<()> {
    print_warn!("The `init` command is not implemented yet.");
    Ok(())
}
