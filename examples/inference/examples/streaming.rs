//! Streaming input *and* output, in-process, using the typed SDK API.
//!
//! Run with: `cargo run -p inference-example --example streaming`
//!
//! A producer thread feeds prompts into the job over time (streaming input); the inference emits a
//! token at a time (streaming output). The timestamps show that outputs come back as inputs arrive,
//! not after the whole input is collected.

use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use inference_example::{Prompt, WordTokenizer};
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    // Offline connection: no credentials needed. Per-request telemetry is recorded locally (stubbed).
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(80)),
    );

    let start = Instant::now();

    // Streaming INPUT: a producer feeds prompts into the job over time. `InferenceJob::stream`
    // accepts any iterator; an `mpsc::Receiver` is a blocking iterator that yields items as they are
    // sent, and ends when the sender is dropped.
    let (tx, rx) = mpsc::channel::<Prompt>();
    thread::spawn(move || {
        for text in ["the quick brown fox", "jumps over", "the lazy dog"] {
            thread::sleep(Duration::from_millis(600));
            println!("[{:>5}ms] >>> feeding: {text:?}", start.elapsed().as_millis());
            if tx.send(Prompt { text: text.into() }).is_err() {
                return;
            }
        }
        // Dropping `tx` ends the input stream, which lets the inference finish.
    });

    // Streaming OUTPUT: consume tokens as they are produced.
    let stream = job.stream(rx)?;
    for output in stream {
        let token = output.map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("[{:>5}ms] <<< token:   {}", start.elapsed().as_millis(), token.token);
    }

    Ok(())
}
