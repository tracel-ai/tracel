//! Optional adapters built on top of the core experiment primitives.
//!
//! Use `training` (behind the `burn` feature) for Burn `train` integration points such as metric
//! logging, checkpoint recording, and cancellation-aware learner interruption.
//!
//! Use [`tracing`] to route `tracing` events into the current experiment.

pub mod tracing;
// Burn's training traits are synchronous, so these adapters block at each edge and exist only
// where a thread can.
#[cfg(all(feature = "burn", not(target_arch = "wasm32")))]
pub mod training;
