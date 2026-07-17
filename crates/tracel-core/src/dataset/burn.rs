use std::sync::Mutex;

use burn::data::dataset::Dataset;
use serde::de::DeserializeOwned;

use super::DatasetModule;

const DEFAULT_PAGE_SIZE: u32 = 256;

#[derive(Debug, Clone, Copy)]
struct IndexEntry {
    entry_idx: u64,
    page_cursor: Option<u64>,
}

pub struct AnnotationDataset<T> {
    module: DatasetModule,
    name: String,
    version: u32,
    page_size: u32,
    index: Mutex<Option<Vec<IndexEntry>>>,
    _marker: std::marker::PhantomData<T>,
}

impl<T> AnnotationDataset<T> {
    pub fn new(module: DatasetModule, name: impl Into<String>, version: u32) -> Self {
        Self::with_page_size(module, name, version, DEFAULT_PAGE_SIZE)
    }

    pub fn with_page_size(
        module: DatasetModule,
        name: impl Into<String>,
        version: u32,
        page_size: u32,
    ) -> Self {
        Self {
            module,
            name: name.into(),
            version,
            page_size,
            index: Mutex::new(None),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T> AnnotationDataset<T>
where
    T: DeserializeOwned,
{
    fn build_index(&self) -> Option<Vec<IndexEntry>> {
        let mut entries = Vec::new();
        let mut cursor = None;
        loop {
            let page_cursor = cursor;
            let page = self
                .module
                .stream_items(&self.name, self.version, page_cursor, Some(self.page_size))
                .ok()?;

            for raw_item in &page.items {
                match serde_json::from_slice::<T>(&raw_item.payload) {
                    Ok(_) => entries.push(IndexEntry {
                        entry_idx: raw_item.entry_idx,
                        page_cursor,
                    }),
                    Err(e) => {
                        tracing::warn!(
                            entry_idx = raw_item.entry_idx,
                            error = %e,
                            "skipping malformed dataset item"
                        );
                    }
                }
            }

            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Some(entries)
    }

    fn index_guard(&self) -> std::sync::MutexGuard<'_, Option<Vec<IndexEntry>>> {
        let mut guard = self.index.lock().unwrap();
        if guard.is_none() {
            *guard = self.build_index();
        }
        guard
    }
}

impl<T> Dataset<T> for AnnotationDataset<T>
where
    T: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Option<T> {
        let entry = {
            let guard = self.index_guard();
            *guard.as_ref()?.get(index)?
        };

        let raw = self
            .module
            .stream_items(
                &self.name,
                self.version,
                entry.page_cursor,
                Some(self.page_size),
            )
            .ok()?;

        raw.items
            .into_iter()
            .find(|raw_item| raw_item.entry_idx == entry.entry_idx)
            .and_then(|raw_item| serde_json::from_slice::<T>(&raw_item.payload).ok())
    }

    fn len(&self) -> usize {
        self.index_guard().as_ref().map(Vec::len).unwrap_or(0)
    }
}

impl DatasetModule {
    /// Wraps this dataset streaming module as a `burn::data::dataset::Dataset<T>`, decoding
    /// each item's payload as JSON.
    pub fn as_burn_dataset<T>(&self, name: impl Into<String>, version: u32) -> AnnotationDataset<T>
    where
        T: DeserializeOwned + Clone + Send + Sync,
    {
        AnnotationDataset::new(self.clone(), name, version)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use burn::data::dataset::Dataset;
    use serde::{Deserialize, Serialize};

    use super::AnnotationDataset;
    use crate::dataset::{
        DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, RawDatasetItem,
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
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
    {
        fn stream_items(
            &self,
            name: &str,
            version: u32,
            cursor: Option<u64>,
            limit: Option<u32>,
        ) -> Result<DatasetItemsPage, DatasetError> {
            (self.stream)(name, version, cursor, limit)
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
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(0, 42)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1);

        assert_eq!(dataset.get(0), Some(TestItem { value: 42 }));
    }

    #[test]
    fn given_malformed_item_when_get_then_it_is_skipped_and_valid_items_still_decode() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
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
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1);

        // The malformed item is excluded from the index entirely, so the dataset's length
        // reflects only the one valid item — index 1 doesn't exist, it isn't a shifted view
        // of the raw page.
        assert_eq!(dataset.len(), 1);
        assert_eq!(dataset.get(0), Some(TestItem { value: 7 }));
        assert_eq!(dataset.get(1), None);
    }

    #[test]
    fn given_dataset_when_len_called_twice_then_provider_is_walked_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, cursor: Option<u64>, _limit: Option<u32>| {
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
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1);

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn given_two_indices_in_same_page_when_get_called_twice_then_provider_is_called_twice() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(DatasetItemsPage {
                    items: vec![item(0, 1), item(1, 2)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> =
            AnnotationDataset::with_page_size(module, "ds", 1, 10);

        assert_eq!(dataset.get(0), Some(TestItem { value: 1 }));
        assert_eq!(dataset.get(1), Some(TestItem { value: 2 }));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn given_index_past_page_contents_when_get_then_none_is_returned() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(0, 1)],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> =
            AnnotationDataset::with_page_size(module, "ds", 1, 10);

        assert_eq!(dataset.get(5), None);
    }

    #[test]
    fn given_transient_error_when_len_then_next_call_retries_instead_of_caching_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                let call = calls_clone.fetch_add(1, Ordering::SeqCst);
                if call == 0 {
                    Err(DatasetError::Client("transient".into()))
                } else {
                    Ok(DatasetItemsPage {
                        items: vec![item(0, 1)],
                        next_cursor: None,
                    })
                }
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1);

        assert_eq!(dataset.len(), 0);
        assert_eq!(dataset.len(), 1);
    }
}
