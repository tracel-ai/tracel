//! A streaming HTTP client for the `server` example: streams prompts over time on the request body
//! and prints tokens as they arrive.
//!
//! Run the server first: `cargo run -p inference-example --example server`
//! Then, in another terminal: `cargo run -p inference-example --example streaming_client`
//!
//! The request body is a stream, so prompts are sent incrementally (unlike `curl -d`, which buffers
//! the whole body first). Timestamps show that the tokens for each prompt come back before the next
//! prompt is sent.

use std::time::{Duration, Instant};

use eventsource_stream::Eventsource;
use inference_example::{Prompt, Token};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let start = Instant::now();

    // Streaming request body: send one prompt every 700ms; reqwest streams them to the server.
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

    // Read the Server-Sent Events response, decoding each token as it arrives.
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
