//! Inference runtime module.
//!
//! This module provides a small abstraction layer for running model inference in three styles:
//! * Direct (single output) – handler returns one value which is collected.
//! * Streaming – handler can emit multiple outputs through an [`OutStream`].
//! * Stateful – user supplied state value is injected into the handler once via the [`State`] param.
//!
//! The core flow is:
//! 1. Build or load a model using [`InferenceBuilder::init`] / [`InferenceBuilder::with_model`].
//! 2. Register a handler (any function/closure convertible to a routine) with `build` producing an [`Inference`].
//! 3. Start a job with [`Inference::infer`], then either `.run()` (collect all outputs) or `.spawn()` (stream them).
//! 4. Optionally cancel a spawned job via [`JobHandle::cancel`].
//!
//! Common re‑exports are provided for convenience.
mod builder;
mod context;
mod core;
mod error;
mod init;
mod job;
mod model;
mod streaming;

#[cfg(test)]
mod tests;

pub use builder::*;
pub use core::*;
pub use error::InferenceError;
pub use init::Init;
pub use job::JobHandle;
pub use model::ModelAccessor;
pub use streaming::{CancelToken, EmitError, Emitter, OutStream};
