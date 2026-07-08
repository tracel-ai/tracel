//! Typed, streaming inference contracts.

pub mod error;
mod inference;
pub mod observer;
mod provider;
pub mod reader;
mod session;
pub mod stream;
mod writer;

pub use error::InferenceError;
pub use inference::{Inference, InferenceFn, InferenceWrapper, inference_fn};
pub use provider::{InferenceJob, InferenceModule, InferenceProvider};
pub use reader::{InferenceInput, InferenceReaderChannel, InferenceReaderError};
pub use session::{InferenceId, InferenceSession};
pub use stream::{DirectInference, InferenceStream};
pub use writer::{InferenceWriter, InferenceWriterChannel, InferenceWriterError};
