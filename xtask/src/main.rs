#[macro_use]
extern crate log;

use std::time::Instant;
use tracel_xtask::prelude::*;

#[macros::base_commands(Build, Bump, Check, Compile, Doc, Fix, Publish, Test, Validate)]
enum Command {}

fn main() -> anyhow::Result<()> {
    let start = Instant::now();
    let args = init_xtask::<Command>(parse_args::<Command>()?)?;
    dispatch_base_commands(args)?;
    let duration = start.elapsed();
    info!(
        "\x1B[32;1mTime elapsed for the current execution: {}\x1B[0m",
        format_duration(&duration)
    );

    Ok(())
}
