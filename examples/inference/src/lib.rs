//! A tiny streaming inference used by the examples: it splits each input prompt into whitespace
//! tokens and streams them back one at a time, with an optional per-token delay.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracel::inference::{Inference, InferenceInput, InferenceSession, InferenceWriter};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Prompt {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
}

pub struct WordTokenizer {
    per_token_delay: Duration,
}

impl WordTokenizer {
    pub fn new() -> Self {
        Self {
            per_token_delay: Duration::ZERO,
        }
    }

    /// Pauses `delay` before each token, to make streaming observable.
    pub fn with_delay(delay: Duration) -> Self {
        Self {
            per_token_delay: delay,
        }
    }
}

impl Default for WordTokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Inference for WordTokenizer {
    type Input = Prompt;
    type Output = Token;

    fn infer(&self, input: InferenceInput<Prompt>, writer: InferenceWriter<Token>) {
        // Present when bound to a telemetry provider (e.g. Cloud), `None` offline.
        let session = InferenceSession::current();

        for prompt in input {
            let words: Vec<&str> = prompt.text.split_whitespace().collect();

            // Explicit metric through the session, with a scoped attribute.
            if let Some(session) = &session {
                session
                    .with_attributes([("prompt_len", prompt.text.len() as u64)])
                    .log_gauge("prompt_tokens", words.len() as f64);
            }

            // Routed to the session by the tracing layer (see the `cloud` example).
            tracing::info!(tokens = words.len(), "tokenizing prompt");

            let mut emitted: u64 = 0;
            for word in words {
                if !self.per_token_delay.is_zero() {
                    std::thread::sleep(self.per_token_delay);
                }
                if writer
                    .write(Token {
                        token: word.to_string(),
                    })
                    .is_err()
                {
                    // The consumer disconnected; stop early.
                    return;
                }
                emitted += 1;
            }

            if let Some(session) = &session {
                session.log_counter("tokens_emitted", emitted);
            }
        }
    }
}
