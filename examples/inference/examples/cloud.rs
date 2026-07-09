//! Cloud inference with session telemetry. The job name becomes the inference group (auto-created
//! on first request); per-request stats and any metrics/logs from `infer` are shipped to it.
//!
//! Needs cloud config (`TRACEL_API_KEY`/`TRACEL_NAMESPACE`/`TRACEL_PROJECT` or `tracel.toml` +
//! `burn login`). Run with: `cargo run -p inference-example --example cloud`

use inference_example::{Prompt, WordTokenizer};
use tracel::inference::integration::tracing::try_init_tracing_subscriber;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    // Forwards `tracing` events from inside `infer` to the session as scoped logs.
    try_init_tracing_subscriber();

    let module = Context::new(Connection::Cloud)?.inference();
    let job = module.create("wordtok", WordTokenizer::default());

    let stream = job.stream_once(Prompt {
        text: "hello streaming inference world".to_string(),
    })?;

    for output in stream {
        let token = output.map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("{}", token.token);
    }

    // Telemetry flushes on the worker's interval and when the Context is dropped.
    Ok(())
}
