//! `burn::data::dataset::Dataset` adapter over Station's item streaming API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use burn::data::dataset::Dataset;
use serde::de::DeserializeOwned;

use super::{DatasetModule, DatasetRef};

const DEFAULT_PAGE_SIZE: u32 = 256;

/// A `burn::data::dataset::Dataset` view over a Station-hosted dataset version, backed by
/// `DatasetModule`'s page-at-a-time item streaming. Fetched pages are cached for the lifetime
/// of this value so repeated epochs over the same indices do not re-hit the network.
pub struct StationDataset<T> {
    module: DatasetModule,
    dataset_ref: DatasetRef,
    page_size: u32,
    len: OnceLock<usize>,
    page_cache: Mutex<HashMap<u64, Arc<Vec<T>>>>,
}

impl<T> StationDataset<T> {
    pub fn new(module: DatasetModule, dataset_ref: DatasetRef) -> Self {
        Self::with_page_size(module, dataset_ref, DEFAULT_PAGE_SIZE)
    }

    pub fn with_page_size(module: DatasetModule, dataset_ref: DatasetRef, page_size: u32) -> Self {
        Self {
            module,
            dataset_ref,
            page_size,
            len: OnceLock::new(),
            page_cache: Mutex::new(HashMap::new()),
        }
    }

    fn page_start(&self, index: usize) -> u64 {
        (index as u64 / self.page_size as u64) * self.page_size as u64
    }

    fn offset_in_page(&self, index: usize) -> usize {
        index % self.page_size as usize
    }
}

impl<T> Dataset<T> for StationDataset<T>
where
    T: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Option<T> {
        let page_start = self.page_start(index);
        let offset = self.offset_in_page(index);

        if let Some(page) = self.page_cache.lock().unwrap().get(&page_start) {
            return page.get(offset).cloned();
        }

        let raw = self
            .module
            .stream_items(&self.dataset_ref, Some(page_start), Some(self.page_size))
            .ok()?;

        let decoded: Vec<T> = raw
            .items
            .into_iter()
            .filter_map(|raw_item| match serde_json::from_slice::<T>(&raw_item.payload) {
                Ok(value) => Some(value),
                Err(e) => {
                    tracing::warn!(
                        entry_idx = raw_item.entry_idx,
                        error = %e,
                        "skipping malformed dataset item"
                    );
                    None
                }
            })
            .collect();

        let item = decoded.get(offset).cloned();
        self.page_cache
            .lock()
            .unwrap()
            .insert(page_start, Arc::new(decoded));
        item
    }

    fn len(&self) -> usize {
        *self.len.get_or_init(|| {
            let mut count = 0usize;
            let mut cursor = None;
            loop {
                let Ok(page) = self.module.stream_items(&self.dataset_ref, cursor, Some(1000))
                else {
                    break;
                };
                count += page.items.len();
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            count
        })
    }
}

impl DatasetModule {
    /// Wraps this dataset streaming module as a `burn::data::dataset::Dataset<T>`, decoding
    /// each item's payload as JSON.
    pub fn as_burn_dataset<T>(&self, dataset_ref: DatasetRef) -> StationDataset<T>
    where
        T: DeserializeOwned + Clone + Send + Sync,
    {
        StationDataset::new(self.clone(), dataset_ref)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use burn::data::dataset::Dataset;
    use serde::{Deserialize, Serialize};

    use super::StationDataset;
    use crate::dataset::{
        DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, DatasetRef,
        RawDatasetItem,
    };

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
    struct TestItem {
        value: u32,
    }

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

    fn item(entry_idx: u64, value: u32) -> RawDatasetItem {
        RawDatasetItem {
            entry_idx,
            payload: serde_json::to_vec(&TestItem { value }).unwrap(),
        }
    }

    #[test]
    fn given_valid_json_item_when_get_then_item_is_decoded() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(0, 42)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: StationDataset<TestItem> = module.as_burn_dataset(dataset_ref);

        assert_eq!(dataset.get(0), Some(TestItem { value: 42 }));
    }

    #[test]
    fn given_malformed_item_when_get_then_it_is_skipped_and_valid_items_still_decode() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![
                        RawDatasetItem {
                            entry_idx: 0,
                            payload: b"not json".to_vec(),
                        },
                        item(1, 7),
                    ],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: StationDataset<TestItem> = module.as_burn_dataset(dataset_ref);

        // The malformed item is dropped from the decoded page, so the one surviving item
        // shifts down to offset 0; there is no second item in the decoded page.
        assert_eq!(dataset.get(0), Some(TestItem { value: 7 }));
        assert_eq!(dataset.get(1), None);
    }

    #[test]
    fn given_dataset_when_len_called_twice_then_provider_is_walked_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_dataset_ref: &DatasetRef, cursor: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                match cursor {
                    None => Ok(DatasetItemsPage {
                        items: vec![item(0, 1)],
                        next_cursor: Some(1),
                    }),
                    Some(1) => Ok(DatasetItemsPage {
                        items: vec![item(1, 2)],
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
        let dataset: StationDataset<TestItem> = module.as_burn_dataset(dataset_ref);

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.len(), 2);
        // Two pages walked on the first call, zero more on the second.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn given_two_indices_in_same_page_when_get_called_twice_then_provider_is_called_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(DatasetItemsPage {
                    items: vec![item(0, 1), item(1, 2)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: StationDataset<TestItem> =
            StationDataset::with_page_size(module, dataset_ref, 10);

        assert_eq!(dataset.get(0), Some(TestItem { value: 1 }));
        assert_eq!(dataset.get(1), Some(TestItem { value: 2 }));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn given_index_past_page_contents_when_get_then_none_is_returned() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(0, 1)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: StationDataset<TestItem> =
            StationDataset::with_page_size(module, dataset_ref, 10);

        assert_eq!(dataset.get(5), None);
    }
}
