//! Streaming access to datasets registered in Tracel Station.
//!
//! Datasets are versioned collections of items stored in Station. This module lets you
//! stream items from a dataset version, page by page, without downloading the whole thing
//! up front.
//!
//! # Getting a dataset
//!
//! Obtain a [`DatasetModule`] from [`Context::datasets`](crate::Context::datasets). It
//! returns `None` unless the context is connected to Station, since dataset streaming is a
//! Station-only feature. [`DatasetModule`] exposes two Burn dataloader adapters — its only
//! supported entry points — which decode each item as JSON into the type you ask for:
//!
//! - [`DatasetModule::stream`] fetches each item from Station on demand, with nothing
//!   written to disk.
//! - [`DatasetModule::download`] downloads the whole dataset version to a local on-disk
//!   cache the first time it's requested, then serves every later access from that file
//!   with no further network calls.
//!
//! Items must be JSON-encoded on the Station side; there is currently no way to plug in a
//! custom [`Dataset`](burn::data::dataset::Dataset) implementation or a different decode
//! format.
//!
//! Versions are selected with a [`DatasetVersionSpec`]: [`DatasetVersionSpec::Fixed`] for an
//! exact version, or [`DatasetVersionSpec::Latest`] to resolve whichever version is newest.

mod download;
#[cfg(feature = "station")]
mod station;
mod stream;

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

/// Errors that can occur while resolving or streaming a dataset.
#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    /// No dataset with this name is registered in Station.
    #[error("dataset '{name}' not found")]
    DatasetNotFound { name: String },
    /// The dataset exists, but not the requested version.
    #[error("version {version} of dataset '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    /// The dataset exists but has no published versions, so [`DatasetVersionSpec::Latest`]
    /// could not be resolved.
    #[error("dataset '{name}' has no versions")]
    NoVersionsFound { name: String },
    /// The underlying Station client failed to communicate with the dataset registry.
    #[error("communication with the dataset registry failed: {0}")]
    Client(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// An item could not be decoded into the type requested by the caller.
    #[error("item {index} in dataset '{name}' version {version} is corrupt: {source}")]
    CorruptItem {
        name: String,
        version: u32,
        index: u64,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// An item read from a downloaded dataset's local cache file could not be decoded.
    /// Unlike [`DatasetError::CorruptItem`], this means the on-disk cache itself is bad,
    /// not the data Station sent — the message tells the caller which directory to delete
    /// to force a fresh download.
    #[error(
        "item {index} in downloaded dataset '{name}' version {version} is corrupt: {source}. \
         The local cache may be corrupted — delete {path} and download the dataset again."
    )]
    CorruptCachedItem {
        name: String,
        version: u32,
        index: u64,
        path: PathBuf,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// No local cache directory could be resolved for a downloaded dataset (e.g. no
    /// home directory available).
    #[error("no local cache directory is available for downloaded datasets")]
    CacheUnavailable,
    /// Reading or writing the local dataset cache on disk failed.
    #[error("local dataset cache I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Selects which version of a dataset to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetVersionSpec {
    /// Use this exact version number.
    Fixed(u32),
    /// Resolve and use whichever version is newest at the time of the call.
    Latest,
}

/// One page of raw items returned by a [`DatasetProvider`].
#[derive(Debug, Clone)]
pub struct DatasetItemsPage {
    /// The raw item bytes, in the order returned by the backend.
    pub items: Vec<Vec<u8>>,
}

/// Backend used by [`DatasetModule`] to resolve versions and stream dataset items.
///
/// This is implemented once per backend (e.g. Station) and is not meant to be called
/// directly by SDK users — go through [`DatasetModule`] instead.
pub trait DatasetProvider: Send + Sync {
    /// Fetches one page of raw items for the named dataset version, starting at `index`
    /// (`None` for the first item) and capped at `limit` items (backend-defined default if
    /// `None`). `index` addresses items directly: fetching with `index = Some(3)` and
    /// `limit = Some(1)` returns the single item at position 3.
    fn get_items(
        &self,
        name: &str,
        version: u32,
        index: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError>;

    /// Returns the total number of items in the named dataset version.
    fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError>;

    /// Resolves the latest version number for the named dataset.
    fn resolve_version(&self, name: &str) -> Result<u32, DatasetError>;
}

/// Entry point for reading datasets registered in Tracel Station.
///
/// Obtain one from [`Context::datasets`](crate::Context::datasets), then hand it to
/// [`DatasetModule::stream`] or [`DatasetModule::download`] to get a Burn `Dataset` —
/// either method's docs include a full usage example.
#[derive(Clone)]
pub struct DatasetModule {
    provider: Arc<dyn DatasetProvider>,
}

impl DatasetModule {
    /// Wraps a dataset backend into a `DatasetModule`. SDK users normally get a
    /// `DatasetModule` from [`Context::datasets`](crate::Context::datasets) instead of
    /// calling this directly.
    pub fn new(provider: Arc<dyn DatasetProvider>) -> Self {
        Self { provider }
    }

    pub(crate) fn get_items(
        &self,
        name: &str,
        version: u32,
        index: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError> {
        self.provider.get_items(name, version, index, limit)
    }

    pub(crate) fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
        self.provider.item_count(name, version)
    }

    pub(crate) fn resolve_version(
        &self,
        name: &str,
        spec: DatasetVersionSpec,
    ) -> Result<u32, DatasetError> {
        match spec {
            DatasetVersionSpec::Fixed(version) => Ok(version),
            DatasetVersionSpec::Latest => self.provider.resolve_version(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider<F> {
        stream: F,
    }

    impl<F> DatasetProvider for FakeProvider<F>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
    {
        fn get_items(
            &self,
            name: &str,
            version: u32,
            index: Option<u64>,
            limit: Option<u32>,
        ) -> Result<DatasetItemsPage, DatasetError> {
            (self.stream)(name, version, index, limit)
        }

        fn item_count(&self, _name: &str, _version: u32) -> Result<u64, DatasetError> {
            Ok(0)
        }

        fn resolve_version(&self, _name: &str) -> Result<u32, DatasetError> {
            Ok(0)
        }
    }

    #[test]
    fn given_exact_spec_when_resolve_version_then_provider_is_not_called() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                panic!("stream should not be called")
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let version = module
            .resolve_version("mnist-corrections", DatasetVersionSpec::Fixed(7))
            .unwrap();

        assert_eq!(version, 7);
    }

    #[test]
    fn given_latest_spec_when_resolve_version_then_provider_resolves_it() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                panic!("stream should not be called")
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let version = module
            .resolve_version("mnist-corrections", DatasetVersionSpec::Latest)
            .unwrap();

        assert_eq!(version, 0);
    }

    #[test]
    fn given_provider_returns_page_when_stream_items_then_page_is_returned() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![b"hello".to_vec()],
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let page = module
            .get_items("mnist-corrections", 1, None, None)
            .unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0], b"hello");
    }

    #[test]
    fn given_provider_returns_not_found_when_stream_items_then_error_is_propagated() {
        let provider = FakeProvider {
            stream: |name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Err(DatasetError::DatasetNotFound {
                    name: name.to_string(),
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let result = module.get_items("mnist-corrections", 1, None, None);

        assert!(matches!(
            result,
            Err(DatasetError::DatasetNotFound { name }) if name == "mnist-corrections"
        ));
    }
}
