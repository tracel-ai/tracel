#[cfg(feature = "burn")]
mod burn_integration;
#[cfg(feature = "station")]
mod station;

#[cfg(feature = "burn")]
pub use burn_integration::AnnotationDataset;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A page of decoded annotation items, as returned to callers of [`DatasetModule`].
#[derive(Debug, Clone)]
pub struct AnnotationPage {
    pub items: Vec<AnnotationItem>,
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

    /// Fetches one page of decoded annotation items, starting at `cursor` (`None` for the first
    /// page) and capped at `limit` items (backend-defined default if `None`). Items that fail to
    /// decode as [`AnnotationItem`] are skipped and logged rather than failing the whole page.
    pub fn stream_items(
        &self,
        dataset_ref: &DatasetRef,
        cursor: Option<u64>,
        limit: Option<u32>,
    ) -> Result<AnnotationPage, DatasetError> {
        let raw = self.provider.stream_items(dataset_ref, cursor, limit)?;

        let items = raw
            .items
            .into_iter()
            .filter_map(|raw_item| {
                match serde_json::from_slice::<AnnotationItem>(&raw_item.payload) {
                    Ok(item) => Some(item),
                    Err(e) => {
                        tracing::warn!(
                            entry_idx = raw_item.entry_idx,
                            error = %e,
                            "skipping malformed dataset item"
                        );
                        None
                    }
                }
            })
            .collect();

        Ok(AnnotationPage {
            items,
            next_cursor: raw.next_cursor,
        })
    }
}

const DEFAULT_PAGE_SIZE: u32 = 256;

struct DatasetIndex {
    len: usize,
    page_cursors: Vec<Option<u64>>,
}

pub struct IndexedAnnotationDataset {
    module: DatasetModule,
    dataset_ref: DatasetRef,
    page_size: u32,
    index: OnceLock<DatasetIndex>,
    page_cache: Mutex<HashMap<usize, Arc<Vec<AnnotationItem>>>>,
}

impl IndexedAnnotationDataset {
    pub fn new(module: DatasetModule, dataset_ref: DatasetRef) -> Self {
        Self::with_page_size(module, dataset_ref, DEFAULT_PAGE_SIZE)
    }

    pub fn with_page_size(module: DatasetModule, dataset_ref: DatasetRef, page_size: u32) -> Self {
        Self {
            module,
            dataset_ref,
            page_size,
            index: OnceLock::new(),
            page_cache: Mutex::new(HashMap::new()),
        }
    }

    /// Walks the dataset once, recording the item count and the real cursor at each page
    /// boundary. Shared by `len` and `get` so neither triggers its own separate walk. Every page
    /// fetched during the walk is stashed in the page cache too, so a `get` right after does not
    /// re-fetch a page this walk already pulled.
    fn index(&self) -> &DatasetIndex {
        self.index.get_or_init(|| {
            let mut len = 0usize;
            let mut page_cursors = Vec::new();
            let mut cursor = None;
            let mut page_index = 0usize;
            loop {
                page_cursors.push(cursor);
                let Ok(page) =
                    self.module
                        .stream_items(&self.dataset_ref, cursor, Some(self.page_size))
                else {
                    break;
                };
                len += page.items.len();
                let next_cursor = page.next_cursor;
                self.page_cache
                    .lock()
                    .unwrap()
                    .insert(page_index, Arc::new(page.items));
                match next_cursor {
                    Some(next) => {
                        cursor = Some(next);
                        page_index += 1;
                    }
                    None => break,
                }
            }
            DatasetIndex { len, page_cursors }
        })
    }

    pub fn len(&self) -> usize {
        self.index().len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, position: usize) -> Option<AnnotationItem> {
        let page_index = position / self.page_size as usize;
        let offset = position % self.page_size as usize;
        let cursor = *self.index().page_cursors.get(page_index)?;

        if let Some(page) = self.page_cache.lock().unwrap().get(&page_index) {
            return page.get(offset).cloned();
        }

        let page = self
            .module
            .stream_items(&self.dataset_ref, cursor, Some(self.page_size))
            .ok()?;

        let item = page.items.get(offset).cloned();
        self.page_cache
            .lock()
            .unwrap()
            .insert(page_index, Arc::new(page.items));
        item
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

    fn annotation_item_payload(example_payload: &[u8]) -> Vec<u8> {
        serde_json::to_vec(&AnnotationItem {
            source_item_id: None,
            example_payload: example_payload.to_vec(),
            example_size_bytes: example_payload.len() as u64,
            annotation: None,
        })
        .unwrap()
    }

    #[test]
    fn given_provider_returns_page_when_stream_items_then_page_is_decoded() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![RawDatasetItem {
                        entry_idx: 0,
                        payload: annotation_item_payload(b"hello"),
                    }],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("mnist-corrections".to_string(), 1);

        let page = module.stream_items(&dataset_ref, None, None).unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].example_payload, b"hello");
        assert!(page.next_cursor.is_none());
    }

    #[test]
    fn given_malformed_item_when_stream_items_then_it_is_skipped() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![
                        RawDatasetItem {
                            entry_idx: 0,
                            payload: b"not json".to_vec(),
                        },
                        RawDatasetItem {
                            entry_idx: 1,
                            payload: annotation_item_payload(b"world"),
                        },
                    ],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("mnist-corrections".to_string(), 1);

        let page = module.stream_items(&dataset_ref, None, None).unwrap();

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].example_payload, b"world");
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

    fn raw_item(entry_idx: u64, example_payload: &[u8]) -> RawDatasetItem {
        RawDatasetItem {
            entry_idx,
            payload: annotation_item_payload(example_payload),
        }
    }

    #[test]
    fn given_indexed_dataset_when_len_called_twice_then_provider_is_walked_only_once() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_dataset_ref: &DatasetRef, cursor: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match cursor {
                    None => Ok(DatasetItemsPage {
                        items: vec![raw_item(0, b"a")],
                        next_cursor: Some(1),
                    }),
                    Some(1) => Ok(DatasetItemsPage {
                        items: vec![raw_item(1, b"b")],
                        next_cursor: None,
                    }),
                    _ => Ok(DatasetItemsPage {
                        items: vec![],
                        next_cursor: None,
                    }),
                }
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset = IndexedAnnotationDataset::new(module, dataset_ref);

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.len(), 2);
        // Two pages walked to build the index once, zero more on the second `len` call.
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn given_two_positions_in_same_page_when_get_called_twice_then_provider_is_called_once() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(DatasetItemsPage {
                    items: vec![raw_item(0, b"a"), raw_item(1, b"b")],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset = IndexedAnnotationDataset::with_page_size(module, dataset_ref, 10);

        assert_eq!(dataset.get(0).unwrap().example_payload, b"a");
        assert_eq!(dataset.get(1).unwrap().example_payload, b"b");
        // The first `get` builds the index, which walks (and caches) the one page this small
        // dataset has; the second `get` must be served entirely from that cache.
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn given_position_past_dataset_end_when_get_then_none_is_returned() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![raw_item(0, b"a")],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset = IndexedAnnotationDataset::with_page_size(module, dataset_ref, 10);

        assert!(dataset.get(5).is_none());
    }

    #[test]
    fn given_dataset_spanning_multiple_pages_when_get_at_second_page_then_correct_item_is_returned()
    {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, cursor: Option<u64>, _limit: Option<u32>| {
                match cursor {
                    None => Ok(DatasetItemsPage {
                        items: vec![raw_item(0, b"a"), raw_item(1, b"b")],
                        next_cursor: Some(2),
                    }),
                    Some(2) => Ok(DatasetItemsPage {
                        items: vec![raw_item(2, b"c")],
                        next_cursor: None,
                    }),
                    _ => Ok(DatasetItemsPage {
                        items: vec![],
                        next_cursor: None,
                    }),
                }
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset = IndexedAnnotationDataset::with_page_size(module, dataset_ref, 2);

        assert_eq!(dataset.len(), 3);
        assert_eq!(dataset.get(2).unwrap().example_payload, b"c");
    }
}
