//! Serve a streaming inference over HTTP (Server-Sent Events), with a streaming request body.
//!
//! Run with:   `cargo run -p inference-example --example server`
//!
//! Single-shot request (one prompt in, tokens streamed out):
//!   `curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'`
//!
//! Streaming request (many prompts fed over time): run the `streaming_client` example, which sends
//! NDJSON prompts on a chunked body and prints tokens as they arrive.

use std::time::Duration;

use inference_example::WordTokenizer;
use tracel::app::server::{JsonBody, Server};
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(150)),
    );

    Server::new()
        .port(3000)
        .register(job, JsonBody::new())
        .run()?;

    Ok(())
}
