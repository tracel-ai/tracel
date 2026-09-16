//! Streaming inference in-process: prompts fed over time, tokens streamed back.
//!
//! cargo run -p basics --example infer

use std::thread;
use std::time::{Duration, Instant};

use basics::{Prompt, WordTokenizer};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let module = common::context()?.inference();
    let job = module.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(120)),
    );

    let (tx, rx) = std::sync::mpsc::channel::<Prompt>();
    let start = Instant::now();
    thread::spawn(move || {
        for text in ["the quick brown fox", "jumps over", "the lazy dog"] {
            thread::sleep(Duration::from_millis(500));
            if tx
                .send(Prompt {
                    text: text.to_string(),
                })
                .is_err()
            {
                return;
            }
        }
    });

    // The inference computes on its own thread; tokens are pulled here as they are produced.
    let (job, tokens) = job.stream(rx);
    let inference = thread::spawn(move || job.block());
    for item in tokens.blocking_iter() {
        let token = item?;
        println!("[{:>5}ms] {}", start.elapsed().as_millis(), token.token);
    }
    inference.join().expect("the inference thread panicked")?;

    Ok(())
}
