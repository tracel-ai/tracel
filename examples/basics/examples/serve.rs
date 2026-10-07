//! An HTTP server serving both jobs: SSE for inference, fire-and-forget for training.
//!
//! cargo run -p basics --example serve
//! curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
//! curl -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
//!
//! For a streaming request, run the infer-client example.

use std::time::Duration;

use basics::WordTokenizer;
use basics::training::{self, TrainingConfig};
use tracel::Target;
use tracel::app::server::{JsonBody, Server};

fn main() -> anyhow::Result<()> {
    let target = Target::from_env()?;

    let infer = target.inference()?.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(120)),
    );
    let train = target
        .experiments()?
        .create("toy-training", training::train);

    Server::new()
        .port(3000)
        .register(infer, JsonBody::new())
        .register(train, JsonBody::with_default(TrainingConfig::default()))
        .run()?;

    Ok(())
}
