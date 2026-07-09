//! Typed, streaming inference contracts.

pub mod context;
pub mod error;
mod inference;
pub mod integration;
pub mod observer;
mod provider;
pub mod reader;
mod session;
pub mod sink;
pub mod stream;
mod writer;

pub use context::SessionGuard;
pub use error::InferenceError;
pub use inference::{Inference, InferenceFn, InferenceWrapper, inference_fn};
pub use provider::{InferenceJob, InferenceModule, InferenceProvider};
pub use reader::{InferenceInput, InferenceReaderChannel, InferenceReaderError};
pub use session::{InferenceId, InferenceSession};
pub use sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricKind, MetricSample,
    NoopSink, now_ms,
};
pub use stream::{DirectInference, InferenceStream};
pub use writer::{InferenceWriter, InferenceWriterChannel, InferenceWriterError};
