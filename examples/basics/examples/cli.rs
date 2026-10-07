//! A CLI serving both jobs. Select one by name and pass its JSON input.
//!
//! cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
//! cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'
//!
//! List the jobs and their inputs instead of running one:
//!
//! TRACEL_DESCRIBE=jobs.json cargo run -p basics --example cli
//!
//! Write the run report of the experiment the job creates:
//!
//! TRACEL_REPORT_FILE=report.json cargo run -p basics --example cli -- toy-training

use std::process::ExitCode;

use basics::training::{self, TrainingConfig};
use basics::{Prompt, WordTokenizer};
use tracel::Target;
use tracel::app::cli::Cli;
use tracel::app::mapper::JsonMapper;

fn main() -> anyhow::Result<ExitCode> {
    let target = Target::from_env()?;

    let infer = target
        .inference()?
        .create("wordtok", WordTokenizer::default())
        .with_description("Split a prompt into tokens");
    let train = target
        .experiments()?
        .create("toy-training", training::train)
        .with_description("Run a toy training loop");

    Ok(Cli::new()
        .register(infer, JsonMapper::<Prompt>::new().with_schema())
        .register(
            train,
            JsonMapper::with_default(TrainingConfig::default()).with_schema(),
        )
        .run())
}
