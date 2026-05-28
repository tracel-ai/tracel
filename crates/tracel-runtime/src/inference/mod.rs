//! Inference registration and runtime integration.

mod fleet;
mod registry;

pub use tracel_inference::*;
pub use fleet::build_fleet_managed_inference;
pub use registry::{InferenceArgs, InferenceError, InferenceInit, InferenceRegistry, ModelSource};
