#![deny(missing_docs)]

//! Backend-agnostic model domain and capability.
//!
//! [`Models`] owns model transfer, verification, staging, and delivery.
//! Backends implement the five blocking primitives in [`ModelOps`] after binding their own scope.

mod domain;
mod error;
mod models;
mod ops;
#[cfg(test)]
mod test_support;

pub use domain::{Model, ModelVersion, VersionFile, VersionId, VersionManifest};
pub use error::ModelsError;
pub use models::Models;
pub use ops::{ModelOps, VersionFileReader, VersionFileSource};
pub use tracel_artifact::TransferObserver;
pub use tracel_artifact::bundle::{BundleDecode, BundleSink, FsBundle};
