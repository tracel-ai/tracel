//! Streaming HTTP client for the `serve` example: sends prompts over time, prints tokens as they
//! arrive. Start `serve` first, then run this in another terminal.
//!
//! cargo run -p basics --example infer-client

use std::time::{Duration, Instant};

use basics::{Prompt, Token};
use eventsource_stream::Eventsource;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = Instant::now();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(8);
    tokio::spawn(async move {
        for text in ["the quick brown fox", "jumps over", "the lazy dog"] {
            tokio::time::sleep(Duration::from_millis(700)).await;
            let mut line = serde_json::to_vec(&Prompt { text: text.into() }).unwrap();
            line.push(b'\n');
            if tx.send(Ok(line)).await.is_err() {
                return;
            }
        }
    });

    let response = reqwest::Client::new()
        .post("http://localhost:3000/wordtok")
        .body(reqwest::Body::wrap_stream(ReceiverStream::new(rx)))
        .send()
        .await?;

    let mut events = response.bytes_stream().eventsource();
    while let Some(event) = events.next().await {
        let event = event?;
        if event.event == "done" {
            break;
        }
        let token: Token = serde_json::from_str(&event.data)?;
        println!("[{:>5}ms] {}", start.elapsed().as_millis(), token.token);
    }

    Ok(())
}
