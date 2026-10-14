#![deny(missing_docs)]

//! Backend-independent model operations.
//!
//! [`ModelRegistry`] validates, transfers, stages, and loads model versions. Backends implement the
//! blocking operations in [`ModelOps`] for a specific scope.

mod domain;
mod error;
mod ops;
mod registry;
#[cfg(test)]
mod test_support;

pub use domain::{
    Model, ModelVersion, VersionFile, VersionId, VersionManifest, VersionSpec, VersionState,
};
pub use error::ModelsError;
pub use ops::{ModelOps, VersionFileReader, VersionFileSource};
pub use registry::ModelRegistry;
