#[cfg(feature = "station")]
mod station;
#[cfg(feature = "burn")]
mod burn_integration;

#[cfg(feature = "burn")]
pub use burn_integration::StationDataset;

use std::sync::Arc;

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    #[error("dataset '{name}' not found")]
    DatasetNotFound { name: String },
    #[error("version {version} of dataset '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    #[error("communication with the dataset registry failed: {0}")]
    Client(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug, Clone)]
pub struct DatasetRef {
    pub name: String,
    pub version: u32,
}

impl DatasetRef {
    pub fn new(name: String, version: u32) -> Self {
        Self { name, version }
    }
}

/// Specific item serialization format for a dataset of type "annotation_set".
#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize)]
pub struct AnnotationItem {
    pub source_item_id: Option<String>,
    #[serde_as(as = "serde_with::base64::Base64")]
    pub example_payload: Vec<u8>,
    pub example_size_bytes: u64,
    pub annotation: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct RawDatasetItem {
    pub entry_idx: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct DatasetItemsPage {
    pub items: Vec<RawDatasetItem>,
    pub next_cursor: Option<u64>,
}

pub trait DatasetProvider: Send + Sync {
    /// Fetches one page of raw items for `dataset_ref`, starting at `cursor` (`None` for the
    /// first page) and capped at `limit` items (backend-defined default if `None`).
    fn stream_items(
        &self,
        dataset_ref: &DatasetRef,
        cursor: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError>;
}

#[derive(Clone)]
pub struct DatasetModule {
    provider: Arc<dyn DatasetProvider>,
}

impl DatasetModule {
    pub fn new(provider: Arc<dyn DatasetProvider>) -> Self {
        Self { provider }
    }

    pub fn stream_items(
        &self,
        dataset_ref: &DatasetRef,
        cursor: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError> {
        self.provider.stream_items(dataset_ref, cursor, limit)
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
        F: Fn(&DatasetRef, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
    {
        fn stream_items(
            &self,
            dataset_ref: &DatasetRef,
            cursor: Option<u64>,
            limit: Option<u32>,
        ) -> Result<DatasetItemsPage, DatasetError> {
            (self.stream)(dataset_ref, cursor, limit)
        }
    }

    #[test]
    fn given_provider_returns_page_when_stream_items_then_page_is_returned() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![RawDatasetItem {
                        entry_idx: 0,
                        payload: b"hello".to_vec(),
                    }],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("mnist-corrections".to_string(), 1);

        let page = module.stream_items(&dataset_ref, None, None).unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].payload, b"hello");
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn given_provider_returns_not_found_when_stream_items_then_error_is_propagated() {
        let provider = FakeProvider {
            stream: |dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Err(DatasetError::DatasetNotFound {
                    name: dataset_ref.name.clone(),
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("mnist-corrections".to_string(), 1);

        let result = module.stream_items(&dataset_ref, None, None);

        assert!(matches!(
            result,
            Err(DatasetError::DatasetNotFound { name }) if name == "mnist-corrections"
        ));
    }
}
