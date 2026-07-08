//! A tiny streaming inference used by the examples: it splits each input prompt into whitespace
//! tokens and streams them back one at a time, with an optional per-token delay.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracel::inference::{Inference, InferenceInput, InferenceWriter};

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
                    // The consumer disconnected; stop early.
                    return;
                }
            }
        }
    }
}
