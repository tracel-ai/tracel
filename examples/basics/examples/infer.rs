//! Streaming inference in-process: prompts fed over time, tokens streamed back.
//!
//! cargo run -p basics --example infer

use std::thread;
use std::time::{Duration, Instant};

use basics::{Prompt, WordTokenizer};
use tracel::{Connection, Context};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
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

    for item in job.stream(rx)? {
        let token = item?;
        println!("[{:>5}ms] {}", start.elapsed().as_millis(), token.token);
    }

    Ok(())
}
