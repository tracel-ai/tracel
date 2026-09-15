#[macro_use]
extern crate log;

use std::time::Instant;
use tracel_xtask::prelude::*;

mod targets;

#[macros::base_commands]
enum Command {
    /// Build for every target and feature set the SDK supports beyond the host.
    Targets,
}

fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let (args, environment) = init_xtask::<Command>(parse_args::<Command>()?)?;
    match args.command {
        Command::Targets => targets::handle_command()?,
        _ => dispatch_base_commands(args, environment)?,
    }
    let duration = start.elapsed();
    info!(
        "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
        format_duration(&duration)
    );

    Ok(())
}
