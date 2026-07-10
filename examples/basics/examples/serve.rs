//! One HTTP server exposing both capabilities, registered uniformly with `Server::register`:
//! inference streams back as Server-Sent Events, training responds fire-and-forget.
//!
//! Run: `cargo run -p basics --example serve`
//!   curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
//!   curl    -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
//!
//! For a true streaming inference request (prompts over time), run the `infer-client` example.

use std::time::Duration;

use basics::WordTokenizer;
use basics::training::{self, TrainingConfig};
use tracel::app::server::{JsonBody, Server};
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let context = Context::new(Connection::Offline("./runs".into()))?;

    let infer = context.inference().create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(120)),
    );
    let train = context
        .experiment()
        .create("toy-training", |run: &ExperimentRun, config| {
            training::train(run, config)
        });

    Server::new()
        .port(3000)
        .register(infer, JsonBody::new())
        .register(train, JsonBody::with_default(TrainingConfig::default()))
        .run()?;

    Ok(())
}
