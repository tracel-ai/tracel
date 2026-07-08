//! A tiny streaming inference used by the examples.
//!
//! It "generates" by splitting each input prompt into whitespace tokens and streaming them back one
//! at a time — a stand-in for a real token-by-token model. An optional per-token delay simulates
//! generation latency so streaming is observable. State (here, just the delay) is owned by the
//! implementor and accessed through `&self`, so one instance serves many concurrent requests.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracel::inference::{Inference, InferenceInput, InferenceWriter};

/// A prompt to "generate" from.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Prompt {
    pub text: String,
}

/// One streamed output token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Token {
    pub token: String,
}

/// Streams each whitespace-separated word of every input prompt as a separate output token,
/// optionally pausing between tokens to imitate a real model's generation latency.
pub struct WordTokenizer {
    per_token_delay: Duration,
}

impl WordTokenizer {
    /// A tokenizer that emits tokens as fast as possible.
    pub fn new() -> Self {
        Self {
            per_token_delay: Duration::ZERO,
        }
    }

    /// A tokenizer that pauses `delay` before each token, to make streaming observable.
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
        // Pulls each prompt as it arrives (streaming input) and writes each token as it is produced
        // (streaming output).
        for prompt in input {
            for word in prompt.text.split_whitespace() {
                if !self.per_token_delay.is_zero() {
                    std::thread::sleep(self.per_token_delay);
                }
                if writer
                    .write(Token {
                        token: word.to_string(),
                    })
                    .is_err()
                {
                    // The consumer went away (cancelled/disconnected); stop early.
                    return;
                }
            }
        }
    }
}
