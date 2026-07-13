//! Typed, streaming inference contracts.

mod context;
mod error;
mod inference;
mod input;
mod observer;
mod output;
mod provider;
mod session;
mod stream;

pub mod integration;
pub mod sink;

pub use context::SessionGuard;
pub use error::InferenceError;
pub use inference::{Inference, IntoInference, inference_fn};
pub use input::InferenceInput;
pub use output::{InferenceOutput, OutputWriter, OutputWriterError};
pub use provider::{InferenceJob, InferenceModule, InferenceProvider};
pub use session::{InferenceId, InferenceSession};
pub use stream::InferenceStream;
