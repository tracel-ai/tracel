#![deny(missing_docs)]

//! Backend-agnostic dataset domain and registry.
//!
//! [`DatasetRegistry`] owns version resolution, item ordering, and annotation decoding.
//! Backends implement the blocking primitives in [`DatasetOps`] after binding their own scope.
//!
//! With the `burn` feature, [`DatasetHandle`] is also a Burn `Dataset`.

mod domain;
mod error;
mod handle;
mod ops;
mod registry;
#[cfg(test)]
mod test_support;

pub use domain::{Dataset, DatasetVersion, Item, NewItem, VersionId, VersionSpec};
pub use error::DatasetsError;
pub use handle::{DatasetHandle, DatasetItem, Items};
pub use ops::{DatasetOps, Publication};
pub use registry::{DatasetRegistry, VersionDraft};
