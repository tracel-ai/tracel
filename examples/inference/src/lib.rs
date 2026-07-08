//! A tiny streaming inference used by the `manual` and `server` examples.
//!
//! It "generates" by splitting each input prompt into whitespace tokens and streaming them back one
//! at a time — a stand-in for a real token-by-token model. State (here, none) is owned by the
//! implementor and accessed through `&self`, so one instance serves many concurrent requests.

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

/// Streams each whitespace-separated word of every input prompt as a separate output token.
pub struct WordTokenizer;

impl Inference for WordTokenizer {
    type Input = Prompt;
    type Output = Token;

    fn infer(&self, input: InferenceInput<Prompt>, writer: InferenceWriter<Token>) {
        for prompt in input {
            for word in prompt.text.split_whitespace() {
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
