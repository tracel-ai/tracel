//! Manual/programmatic inference: build a job and stream its outputs directly.
//!
//! Run with: `cargo run -p inference-example --example manual`

use inference_example::{Prompt, WordTokenizer};
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create("wordtok", WordTokenizer::default());

    let stream = job.stream_once(Prompt {
        text: "hello streaming inference world".to_string(),
    })?;

    for output in stream {
        let token = output.map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("{}", token.token);
    }

    Ok(())
}
