#![deny(missing_docs)]

//! Backend-agnostic dataset domain and capability.
//!
//! [`Datasets`] owns version resolution, item ordering, and annotation decoding.
//! Backends implement the blocking primitives in [`DatasetOps`] after binding their own scope.
//!
//! With the `burn` feature, [`DatasetHandle`] is also a Burn `Dataset`.

mod datasets;
mod domain;
mod error;
mod handle;
mod ops;
#[cfg(test)]
mod test_support;

pub use datasets::{Datasets, VersionDraft};
pub use domain::{Dataset, DatasetVersion, Item, NewItem, VersionId, VersionSpec};
pub use error::DatasetsError;
pub use handle::{DatasetHandle, DatasetItem, Items};
pub use ops::{DatasetOps, Publication};
