//! Optional adapters built on top of the core experiment primitives.
//!
//! Use `training` (behind the `burn` feature) for Burn `train` integration points such as metric
//! logging, checkpoint recording, and cancellation-aware learner interruption.
//!
//! Use [`tracing`] to route `tracing` events into the current experiment.

pub mod tracing;
#[cfg(feature = "burn")]
pub mod training;
