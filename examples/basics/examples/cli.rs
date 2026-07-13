//! A CLI serving both jobs. Select one by name and pass its JSON config.
//!
//! cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
//! cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'

use basics::WordTokenizer;
use basics::training::{self, TrainingConfig};
use tracel::app::cli::Cli;
use tracel::app::cli::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;

fn main() -> anyhow::Result<()> {
    let context = common::context()?;

    let infer = context
        .inference()
        .create("wordtok", WordTokenizer::default());
    let train = context
        .experiment()
        .create("toy-training", |run: &ExperimentRun, config| {
            training::train(run, config)
        });

    Cli::new()
        .register(infer, JsonMapper::new())
        .register(train, JsonMapper::with_default(TrainingConfig::default()))
        .run()?;

    Ok(())
}
