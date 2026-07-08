//! Serve a streaming inference over HTTP (Server-Sent Events).
//!
//! Run with:   `cargo run -p inference-example --example server`
//! Then query: `curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'`
//!
//! Each output token arrives as its own `data:` frame, terminated by a `done` event.

use inference_example::WordTokenizer;
use tracel::app::server::Server;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create("wordtok", WordTokenizer);

    Server::new().port(3000).register(job).run()?;

    Ok(())
}
