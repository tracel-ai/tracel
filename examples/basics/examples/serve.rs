//! An HTTP server serving both jobs: SSE for inference, fire-and-forget for training.
//!
//! cargo run -p basics --example serve
//! curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
//! curl -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
//!
//! For a streaming request, run the infer-client example.

use std::time::Duration;

use basics::training::{self, TrainingConfig};
use basics::{Prompt, WordTokenizer};
use tracel::Target;
use tracel::app::mapper::JsonMapper;
use tracel::app::server::Server;

fn main() -> anyhow::Result<()> {
    let target = Target::from_env()?;

    let infer = target
        .inference()?
        .create(
            "wordtok",
            WordTokenizer::with_delay(Duration::from_millis(120)),
        )
        .with_description("Split a prompt into tokens");
    let train = target
        .experiments()?
        .create("toy-training", training::train)
        .with_description("Run a toy training loop");

    Server::new()
        .port(3000)
        .register(infer, JsonMapper::<Prompt>::new().with_schema())
        .register(
            train,
            JsonMapper::with_default(TrainingConfig::default()).with_schema(),
        )
        .run()?;

    Ok(())
}
