//! Streaming input and output, in-process, using the typed SDK API.
//!
//! Run with: `cargo run -p inference-example --example streaming`

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use inference_example::{Prompt, WordTokenizer};
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(80)),
    );

    let start = Instant::now();

    // Feed prompts over time from another thread; `stream` accepts the receiver as a blocking
    // iterator, and dropping `tx` ends the input stream.
    let (tx, rx) = mpsc::channel::<Prompt>();
    thread::spawn(move || {
        for text in ["the quick brown fox", "jumps over", "the lazy dog"] {
            thread::sleep(Duration::from_millis(600));
            println!("[{:>5}ms] >>> feeding: {text:?}", start.elapsed().as_millis());
            if tx.send(Prompt { text: text.into() }).is_err() {
                return;
            }
        }
    });

    let stream = job.stream(rx)?;
    for output in stream {
        let token = output.map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("[{:>5}ms] <<< token:   {}", start.elapsed().as_millis(), token.token);
    }

    Ok(())
}
