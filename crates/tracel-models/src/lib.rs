#![deny(missing_docs)]

//! Backend-agnostic model domain and capability.
//!
//! [`Models`] owns model transfer, verification, staging, and delivery.
//! Backends implement the five blocking primitives in [`ModelOps`] after binding their own scope.

mod domain;
mod error;
mod models;
mod ops;

pub use domain::{Model, ModelVersion, Page, VersionFile, VersionId, VersionManifest};
pub use error::ModelsError;
pub use models::Models;
pub use ops::{ModelOps, VersionFileReader, VersionFileSource};
pub use tracel_artifact::bundle::{BundleDecode, BundleSink};
pub use tracel_artifact::download::DownloadObserver;

#[cfg(test)]
mod tests;
