mod burn;
#[cfg(feature = "station")]
mod station;

pub use burn::AnnotationDataset;

use std::error::Error;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    #[error("dataset '{name}' not found")]
    DatasetNotFound { name: String },
    #[error("version {version} of dataset '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error("communication with the dataset registry failed: {0}")]
    Client(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("item {index} in dataset '{name}' version {version} is corrupt: {source}")]
    CorruptItem {
        name: String,
        version: u32,
        index: u64,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
}

#[derive(Debug, Clone)]
pub struct DatasetItemsPage {
    pub items: Vec<Vec<u8>>,
}

pub trait DatasetProvider: Send + Sync {
    /// Fetches one page of raw items for the named dataset version, starting at `index`
    /// (`None` for the first item) and capped at `limit` items (backend-defined default if
    /// `None`). `index` addresses items directly: fetching with `index = Some(3)` and
    /// `limit = Some(1)` returns the single item at position 3.
    fn stream_items(
        &self,
        name: &str,
        version: u32,
        index: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError>;

    // Returns the total number of items in the named /dataset version.
    fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError>;
}

#[derive(Clone)]
pub struct DatasetModule {
    provider: Arc<dyn DatasetProvider>,
}

impl DatasetModule {
    pub fn new(provider: Arc<dyn DatasetProvider>) -> Self {
        Self { provider }
    }

    pub(crate) fn stream_items(
        &self,
        name: &str,
        version: u32,
        index: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError> {
        self.provider.stream_items(name, version, index, limit)
    }

    pub(crate) fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
        self.provider.item_count(name, version)
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
        fn stream_items(
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
            .stream_items("mnist-corrections", 1, None, None)
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

        let result = module.stream_items("mnist-corrections", 1, None, None);

        assert!(matches!(
            result,
            Err(DatasetError::DatasetNotFound { name }) if name == "mnist-corrections"
        ));
    }
}
