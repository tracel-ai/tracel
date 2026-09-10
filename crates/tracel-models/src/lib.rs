#![deny(missing_docs)]

//! Backend-independent model operations.
//!
//! [`Models`] validates, transfers, stages, and loads model versions. Backends implement the
//! blocking operations in [`ModelOps`] for a specific scope.

mod domain;
mod error;
mod models;
mod ops;
#[cfg(test)]
mod test_support;

pub use domain::{Model, ModelVersion, VersionFile, VersionId, VersionManifest, VersionSpec};
pub use error::ModelsError;
pub use models::Models;
pub use ops::{ModelOps, VersionFileReader, VersionFileSource};
