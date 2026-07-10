//! Typed, streaming inference contracts.

pub mod context;
pub mod error;
mod inference;
pub mod input;
pub mod integration;
pub mod observer;
mod output;
mod provider;
mod session;
pub mod sink;
pub mod stream;

pub use context::SessionGuard;
pub use error::InferenceError;
pub use inference::{Inference, InferenceFn, IntoInference, IsFn, IsInference, inference_fn};
pub use input::{InferenceInput, InputReader, InputReaderError};
pub use output::{InferenceOutput, OutputWriter, OutputWriterError};
pub use provider::{InferenceJob, InferenceModule, InferenceProvider};
pub use session::{InferenceId, InferenceSession};
pub use sink::{
    InferenceSink, LogLevel, LogSample, MetricData, MetricDescriptor, MetricKind, MetricSample,
    NoopSink, now_ms,
};
pub use stream::InferenceStream;
