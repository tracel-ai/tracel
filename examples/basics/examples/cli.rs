//! A program with two jobs. Select one by name and give its input as flags, built from the
//! input's schema, or as JSON.
//!
//! cargo run -p basics --example cli -- toy-training --help
//! cargo run -p basics --example cli -- toy-training --epochs 2 --batches-per-epoch 4
//! cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'
//! cargo run -p basics --example cli -- wordtok --text "hello streaming world"
//!
//! List the jobs and their inputs instead of running one:
//!
//! TRACEL_DESCRIBE=jobs.json cargo run -p basics --example cli
//!
//! Write the job's run report, which lists the experiments it starts:
//!
//! TRACEL_REPORT_FILE=report.json cargo run -p basics --example cli -- toy-training

use std::process::ExitCode;

use basics::training::{self, TrainingConfig};
use basics::{Prompt, WordTokenizer};
use tracel::Target;
use tracel::app::{BoxError, Job, Jobs};

/// Runs the toy training loop in an experiment, which records the config as its arguments.
fn toy_training(config: TrainingConfig) -> Result<(), BoxError> {
    let experiments = Target::from_env()?.experiments()?;
    experiments.run("toy-training", |run| {
        run.log_args(&config)?;
        training::train(run, config)
    })
}

/// Splits the prompt into tokens and prints them, one per line. It starts no experiment.
fn wordtok(prompt: Prompt) -> Result<(), BoxError> {
    let tokenizer = Target::from_env()?
        .inference()?
        .create("wordtok", WordTokenizer::default());
    for token in tokenizer.stream_once(prompt)? {
        println!("{}", token?.token);
    }
    Ok(())
}

fn main() -> ExitCode {
    Jobs::new()
        .version(env!("CARGO_PKG_VERSION"))
        .job(
            "toy-training",
            Job::new(toy_training)
                .with_description("Run a toy training loop")
                .with_default(TrainingConfig::default())
                .with_schema(),
        )
        .job(
            "wordtok",
            Job::new(wordtok)
                .with_description("Split a prompt into tokens")
                .with_schema(),
        )
        .run()
}
