//! One CLI exposing both capabilities: an inference job and a training job, registered uniformly
//! with `Cli::register`.
//!
//! Run:
//!   cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
//!   cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'

use basics::WordTokenizer;
use basics::training::{self, TrainingConfig};
use tracel::app::cli::Cli;
use tracel::app::cli::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let context = Context::new(Connection::Offline("./runs".into()))?;

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
