//! Burn dataset adapters for datasets registered in Tracel Station.
//!
//! Datasets are versioned collections of item envelopes. Each envelope contains a base64-encoded
//! example payload, an optional JSON annotation, and optional source metadata. This module decodes
//! the example into [`DatasetItem::example`] as raw bytes and deserializes the annotation into the
//! application type selected by the caller. The example payload's byte format and the annotation
//! schema are contracts defined by the application that publishes and consumes the dataset.
//!
//! Obtain a [`DatasetModule`] from [`Context::datasets`](crate::Context::datasets). It provides two
//! Burn [`Dataset`](burn::data::dataset::Dataset) adapters:
//!
//! - [`DatasetModule::stream`] requests an item from Station on each access and writes nothing to
//!   disk.
//! - [`DatasetModule::download`] downloads a complete version into a local cache, then loads its
//!   raw envelopes into memory when the adapter is constructed.
//!
//! Select an exact version with a `u32` or [`DatasetVersionSpec::Fixed`], or use
//! [`DatasetVersionSpec::Latest`] to resolve the newest published version once when constructing
//! the adapter.

mod download;
mod item;
#[cfg(feature = "station")]
mod stream;

use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;

use serde::de::DeserializeOwned;

pub use download::DownloadedDataset;
pub use item::DatasetItem;
pub use stream::StreamedDataset;

/// Errors that can occur while resolving, downloading, or reading a dataset.
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
    Client(#[source] Box<dyn Error + Send + Sync>),
    /// An item's envelope or base64 example payload could not be decoded.
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
    /// not the data Station sent; the message names the file to delete to force a
    /// fresh download.
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
    /// The envelope was valid, but its annotation did not match the caller's annotation type.
    #[error(
        "annotation for item {index} in dataset '{name}' version {version} could not be decoded: \
         the requested annotation type does not match the dataset's annotation schema: {source}"
    )]
    AnnotationDecode {
        name: String,
        version: u32,
        index: u64,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    /// The provider returned no item for an index within the reported dataset length.
    #[error("dataset '{name}' version {version} returned no item at index {index}")]
    MissingItem {
        name: String,
        version: u32,
        index: u64,
    },
    /// A download ended before the provider's reported item count was reached.
    #[error(
        "dataset '{name}' version {version} download was incomplete: expected {expected} items, \
         received {actual}"
    )]
    IncompleteDownload {
        name: String,
        version: u32,
        expected: u64,
        actual: u64,
    },
    /// No local cache directory could be resolved for a downloaded dataset.
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

impl From<u32> for DatasetVersionSpec {
    fn from(version: u32) -> Self {
        Self::Fixed(version)
    }
}

/// One page of raw item envelopes returned by a [`DatasetProvider`].
#[derive(Debug, Clone)]
pub struct DatasetItemsPage {
    /// Raw JSON envelope bytes in backend order.
    pub items: Vec<Vec<u8>>,
}

/// Backend used by [`DatasetModule`] to resolve versions and retrieve dataset items.
///
/// This is implemented once per backend and is not normally called directly by SDK users.
pub trait DatasetProvider: Send + Sync {
    /// Fetches one page of raw item envelopes for the named dataset version, starting at `index`
    /// (`None` for the first item) and capped at `limit` items (backend-defined default if `None`).
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
/// Obtain one from [`Context::datasets`](crate::Context::datasets), then use
/// [`DatasetModule::stream`] or [`DatasetModule::download`] to construct a Burn dataset adapter.
#[derive(Clone)]
pub struct DatasetModule {
    provider: Arc<dyn DatasetProvider>,
}

impl DatasetModule {
    /// Wraps a dataset backend into a `DatasetModule`. SDK users normally get a module from
    /// [`Context::datasets`](crate::Context::datasets) instead.
    pub fn new(provider: Arc<dyn DatasetProvider>) -> Self {
        Self { provider }
    }

    /// Adapts a named dataset version into a Burn [`Dataset`](burn::data::dataset::Dataset).
    /// Each item is fetched from Station on access and nothing is written to disk.
    ///
    /// Pass a `u32` for an exact version or [`DatasetVersionSpec::Latest`] for the newest version.
    /// `A` is the application annotation type and must match the dataset's annotation schema. The
    /// example payload is exposed as raw bytes for the application to decode in its own format.
    ///
    /// ```rust,no_run
    /// use burn::data::dataset::Dataset;
    /// use serde::Deserialize;
    /// use tracel_core::{DatasetItem, DatasetModule, DatasetVersionSpec};
    ///
    /// #[derive(Deserialize, Clone, Debug)]
    /// struct MyAnnotation {
    ///     label: String,
    /// }
    ///
    /// # fn example(datasets: DatasetModule) -> Result<(), Box<dyn std::error::Error>> {
    /// let dataset = datasets
    ///     .stream::<MyAnnotation>("mnist-corrections", DatasetVersionSpec::Latest)?;
    /// let item: DatasetItem<MyAnnotation> = dataset.get(0)?;
    ///
    /// let raw_example: &[u8] = &item.example; // Decode using the application's payload format.
    /// if let Some(annotation) = &item.annotation {
    ///     println!("{}: {} bytes", annotation.label, raw_example.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn stream<A>(
        &self,
        name: impl Into<String>,
        version: impl Into<DatasetVersionSpec>,
    ) -> Result<StreamedDataset<A>, DatasetError>
    where
        A: DeserializeOwned + Clone + Send + Sync,
    {
        let name = name.into();
        let version = resolve_version(self.provider.as_ref(), &name, version.into())?;
        StreamedDataset::new(self.provider.clone(), name, version)
    }

    /// Adapts a named dataset version into a Burn [`Dataset`](burn::data::dataset::Dataset),
    /// downloading the complete version into a local cache on first use.
    ///
    /// Pass a `u32` for an exact version or [`DatasetVersionSpec::Latest`] for the newest version.
    /// The cache file is reused by later constructions; each construction reads its raw item
    /// envelopes into memory once. `A` must match the application's annotation schema, while each
    /// example remains raw bytes for application-defined decoding.
    ///
    /// ```rust,no_run
    /// use burn::data::dataset::Dataset;
    /// use serde::Deserialize;
    /// use tracel_core::{DatasetItem, DatasetModule, DatasetVersionSpec};
    ///
    /// #[derive(Deserialize, Clone, Debug)]
    /// struct MyAnnotation {
    ///     label: String,
    /// }
    ///
    /// # fn example(datasets: DatasetModule) -> Result<(), Box<dyn std::error::Error>> {
    /// let dataset = datasets
    ///     .download::<MyAnnotation>("mnist-corrections", DatasetVersionSpec::Latest)?;
    /// let item: DatasetItem<MyAnnotation> = dataset.get(0)?;
    ///
    /// let raw_example: &[u8] = &item.example; // Decode using the application's payload format.
    /// if let Some(annotation) = &item.annotation {
    ///     println!("{}: {} bytes", annotation.label, raw_example.len());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn download<A>(
        &self,
        name: impl Into<String>,
        version: impl Into<DatasetVersionSpec>,
    ) -> Result<DownloadedDataset<A>, DatasetError>
    where
        A: DeserializeOwned + Clone + Send + Sync,
    {
        let name = name.into();
        let version = resolve_version(self.provider.as_ref(), &name, version.into())?;
        let cache_root = crate::resolve_cache_dir().ok_or(DatasetError::CacheUnavailable)?;
        DownloadedDataset::try_get_or_download(self.provider.clone(), name, version, &cache_root)
    }
}

fn resolve_version(
    provider: &dyn DatasetProvider,
    name: &str,
    spec: DatasetVersionSpec,
) -> Result<u32, DatasetError> {
    match spec {
        DatasetVersionSpec::Fixed(version) => Ok(version),
        DatasetVersionSpec::Latest => provider.resolve_version(name),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use burn::data::dataset::Dataset;
    use serde::Deserialize;

    use super::{
        DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, DatasetVersionSpec,
    };
    use crate::dataset::item::envelope_item;

    #[derive(Clone, Debug, Deserialize, PartialEq)]
    struct TestAnnotation {
        value: u32,
    }

    struct FakeProvider<F, C, L> {
        stream: F,
        count: C,
        latest: L,
    }

    impl<F, C, L> DatasetProvider for FakeProvider<F, C, L>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
        C: Fn(&str, u32) -> Result<u64, DatasetError> + Send + Sync,
        L: Fn(&str) -> Result<u32, DatasetError> + Send + Sync,
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

        fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
            (self.count)(name, version)
        }

        fn resolve_version(&self, name: &str) -> Result<u32, DatasetError> {
            (self.latest)(name)
        }
    }

    #[test]
    fn given_u32_version_when_stream_then_fixed_version_is_used_without_resolution() {
        let provider = FakeProvider {
            stream: |_name: &str, version: u32, _index: Option<u64>, _limit: Option<u32>| {
                assert_eq!(version, 7);
                Ok(DatasetItemsPage {
                    items: vec![envelope_item(
                        b"example",
                        Some(serde_json::json!({ "value": 1 })),
                    )],
                })
            },
            count: |_name: &str, version: u32| {
                assert_eq!(version, 7);
                Ok(1)
            },
            latest: |_name: &str| panic!("fixed version should not be resolved"),
        };
        let module = DatasetModule::new(Arc::new(provider));

        let dataset = module.stream::<TestAnnotation>("ds", 7).unwrap();

        assert_eq!(
            dataset.get(0).unwrap().annotation,
            Some(TestAnnotation { value: 1 })
        );
    }

    #[test]
    fn given_latest_version_when_dataset_is_used_then_version_is_resolved_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: |_name: &str, version: u32, _index: Option<u64>, _limit: Option<u32>| {
                assert_eq!(version, 9);
                Ok(DatasetItemsPage {
                    items: vec![envelope_item(
                        b"example",
                        Some(serde_json::json!({ "value": 2 })),
                    )],
                })
            },
            count: |_name: &str, version: u32| {
                assert_eq!(version, 9);
                Ok(1)
            },
            latest: move |_name: &str| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(9)
            },
        };
        let module = DatasetModule::new(Arc::new(provider));

        let dataset = module
            .stream::<TestAnnotation>("ds", DatasetVersionSpec::Latest)
            .unwrap();

        assert_eq!(dataset.len(), 1);
        assert_eq!(
            dataset.get(0).unwrap().annotation,
            Some(TestAnnotation { value: 2 })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn given_u32_when_converted_then_fixed_version_spec_is_returned() {
        assert_eq!(DatasetVersionSpec::from(4), DatasetVersionSpec::Fixed(4));
    }
}
