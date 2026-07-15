//! `burn::data::dataset::Dataset` adapter over `IndexedAnnotationDataset`.
//!
//! This is a thin translation layer only: all paging, caching, and cursor bookkeeping live in
//! `IndexedAnnotationDataset` (which has no dependency on Burn). This module exists purely to
//! satisfy Burn's index-based `Dataset` trait by delegating to that position-based accessor.

use burn::data::dataset::Dataset;

use super::{AnnotationItem, DatasetModule, DatasetRef, IndexedAnnotationDataset};

pub struct AnnotationDataset {
    inner: IndexedAnnotationDataset,
}

impl AnnotationDataset {
    pub fn new(module: DatasetModule, dataset_ref: DatasetRef) -> Self {
        Self {
            inner: IndexedAnnotationDataset::new(module, dataset_ref),
        }
    }

    pub fn with_page_size(module: DatasetModule, dataset_ref: DatasetRef, page_size: u32) -> Self {
        Self {
            inner: IndexedAnnotationDataset::with_page_size(module, dataset_ref, page_size),
        }
    }
}

impl Dataset<AnnotationItem> for AnnotationDataset {
    fn get(&self, index: usize) -> Option<AnnotationItem> {
        self.inner.get(index)
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

impl DatasetModule {
    /// Wraps this dataset streaming module as a `burn::data::dataset::Dataset<AnnotationItem>`.
    pub fn as_burn_dataset(&self, dataset_ref: DatasetRef) -> AnnotationDataset {
        AnnotationDataset::new(self.clone(), dataset_ref)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use burn::data::dataset::Dataset;

    use super::AnnotationDataset;
    use crate::dataset::{
        AnnotationItem, DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, DatasetRef,
        RawDatasetItem,
    };

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

    fn item(entry_idx: u64, example_payload: &[u8]) -> RawDatasetItem {
        RawDatasetItem {
            entry_idx,
            payload: serde_json::to_vec(&AnnotationItem {
                source_item_id: None,
                example_payload: example_payload.to_vec(),
                example_size_bytes: example_payload.len() as u64,
                annotation: None,
            })
            .unwrap(),
        }
    }

    // These only need to prove `Dataset::get`/`len` correctly delegate to
    // `IndexedAnnotationDataset` — paging/caching behavior itself is exercised by
    // `dataset::tests` against `IndexedAnnotationDataset` directly, with no Burn dependency.

    #[test]
    fn given_burn_dataset_when_get_then_it_delegates_to_indexed_dataset() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(0, b"hello")],
                    next_cursor: None,
                })
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: AnnotationDataset = module.as_burn_dataset(dataset_ref);

        assert_eq!(dataset.get(0).unwrap().example_payload, b"hello");
        assert!(dataset.get(1).is_none());
    }

    #[test]
    fn given_burn_dataset_when_len_then_it_delegates_to_indexed_dataset() {
        let provider = FakeProvider {
            stream: |_dataset_ref: &DatasetRef, cursor: Option<u64>, _limit: Option<u32>| match cursor
            {
                None => Ok(DatasetItemsPage {
                    items: vec![item(0, b"a")],
                    next_cursor: Some(1),
                }),
                Some(1) => Ok(DatasetItemsPage {
                    items: vec![item(1, b"b")],
                    next_cursor: None,
                }),
                _ => Ok(DatasetItemsPage {
                    items: vec![],
                    next_cursor: None,
                }),
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset_ref = DatasetRef::new("ds".to_string(), 1);
        let dataset: AnnotationDataset = AnnotationDataset::with_page_size(module, dataset_ref, 1);

        assert_eq!(dataset.len(), 2);
    }
}
