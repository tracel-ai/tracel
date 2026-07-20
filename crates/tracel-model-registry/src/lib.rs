//! Model registry contracts for the Tracel SDK.
//!
//! A model registry holds a project's versioned models. A version is a bundle of named files
//! (see [`tracel_artifact::bundle`]) plus opaque, app-defined metadata that the registry stores
//! and syncs but never interprets. The registry hands back artifact bytes; turning them into a
//! runnable model is the caller's concern.

mod error;
mod model;
mod provider;

pub use error::ModelRegistryError;
pub use model::{Availability, FileEntry, Manifest, Model, Revision, Version};
pub use provider::{Artifacts, ModelRegistryModule, ModelRegistryProvider, Result};
