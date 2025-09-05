pub mod builder;
pub mod context;
pub mod core;
pub mod emitter;
pub mod errors;
pub mod job;
pub mod provider;

#[cfg(test)]
mod tests;

// Re-export main types for convenience
pub use builder::{InferenceJob, InferenceJobBuilder, StrappedInferenceJobBuilder};
pub use context::InferenceContext;
pub use core::{Inference, InferenceBuilder, LoadedInferenceBuilder};
pub use emitter::{
    CancelToken, CollectEmitter, EmitControl, Emitter, OutStream, SyncChannelEmitter,
};
pub use errors::{InferenceError, ModelProviderError, ModelProviderResult};
pub use job::JobHandle;
pub use provider::ModelProvider;
