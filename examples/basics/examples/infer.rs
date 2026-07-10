//! Programmatic streaming inference, in-process: a producer feeds prompts over time and tokens
//! stream back as each prompt arrives.
//!
//! Run: `cargo run -p basics --example infer`

use std::thread;
use std::time::{Duration, Instant};

use basics::{Prompt, WordTokenizer};
use tracel::{Connection, Context};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Offline records telemetry locally (stubbed). For Cloud, swap the connection:
    //   let module = Context::new(Connection::Cloud)?.inference();
    let module = Context::new(Connection::Offline("./runs".into()))?.inference();
    let job = module.create(
        "wordtok",
        WordTokenizer::with_delay(Duration::from_millis(120)),
    );

    // Feed prompts over time on another thread; outputs stream back as they arrive.
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

    // `stream` spawns a worker and returns a pull iterator. For a single prompt use `stream_once`.
    for item in job.stream(rx)? {
        let token = item?;
        println!("[{:>5}ms] {}", start.elapsed().as_millis(), token.token);
    }

    Ok(())
}
