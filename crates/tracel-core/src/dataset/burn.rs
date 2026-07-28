use burn::data::dataset::Dataset;
use serde::de::DeserializeOwned;

use super::{DatasetError, DatasetModule, DatasetVersionSpec};

pub struct AnnotationDataset<T> {
    module: DatasetModule,
    name: String,
    version: u32,
    len: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<T> AnnotationDataset<T> {
    pub fn new(
        module: DatasetModule,
        name: impl Into<String>,
        spec: impl Into<DatasetVersionSpec>,
    ) -> Result<Self, DatasetError> {
        let name = name.into();
        let spec = spec.into();
        let version = module.resolve_version(&name, spec)?;
        let len = module.item_count(&name, version)? as usize;

        Ok(Self {
            module,
            name,
            version,
            len,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<T> Dataset<T, DatasetError> for AnnotationDataset<T>
where
    T: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Result<T, DatasetError> {
        let len = self.len();
        assert!(
            index < len,
            "Index out of bounds for AnnotationDataset: {index} >= {len}"
        );

        let version = self.version;
        let page = self
            .module
            .stream_items(&self.name, version, Some(index as u64), Some(1))?;

        let raw_item = page
            .items
            .into_iter()
            .next()
            .expect("provider returned no item for an index within the dataset length");

        serde_json::from_slice::<T>(&raw_item).map_err(|err| DatasetError::CorruptItem {
            name: self.name.clone(),
            version,
            index: index as u64,
            source: Box::new(err),
        })
    }

    fn len(&self) -> usize {
        self.len
    }
}

impl DatasetModule {
    pub fn as_burn_dataset<T>(
        &self,
        name: impl Into<String>,
        spec: impl Into<DatasetVersionSpec>,
    ) -> Result<AnnotationDataset<T>, DatasetError>
    where
        T: DeserializeOwned + Clone + Send + Sync,
    {
        AnnotationDataset::new(self.clone(), name, spec)
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
        DatasetError, DatasetItemsPage, DatasetModule, DatasetProvider, DatasetVersionSpec,
    };

    #[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
    struct TestItem {
        value: u32,
    }

    struct FakeProvider<F, C> {
        stream: F,
        count: C,
    }

    impl<F, C> DatasetProvider for FakeProvider<F, C>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
        C: Fn(&str, u32) -> Result<u64, DatasetError> + Send + Sync,
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

        fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
            (self.count)(name, version)
        }

        fn resolve_version(&self, _name: &str) -> Result<u32, DatasetError> {
            unimplemented!("tests in this module only use DatasetVersionSpec::Exact")
        }
    }

    fn item(value: u32) -> Vec<u8> {
        serde_json::to_vec(&TestItem { value }).unwrap()
    }

    #[test]
    fn given_valid_json_item_when_get_then_item_is_decoded() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(42)],
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1).unwrap();

        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 42 });
    }

    #[test]
    fn given_malformed_item_when_get_then_error_is_returned_and_valid_item_still_decodes() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![match cursor {
                        Some(1) => item(7),
                        _ => b"not json".to_vec(),
                    }],
                })
            },
            count: |_name: &str, _version: u32| Ok(2),
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1).unwrap();

        assert_eq!(dataset.len(), 2);
        assert!(dataset.get(0).is_err());
        assert_eq!(dataset.get(1).unwrap(), TestItem { value: 7 });
    }

    #[test]
    fn given_dataset_when_len_called_twice_then_item_count_is_fetched_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage { items: vec![] })
            },
            count: move |_name: &str, _version: u32| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(2)
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module.as_burn_dataset("ds", 1).unwrap();

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn given_two_indices_when_get_called_twice_then_index_matches_indices() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let all_items = vec![item(1), item(2)];
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, cursor: Option<u64>, limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = cursor.unwrap_or(0) as usize;
                let limit = limit.map(|l| l as usize).unwrap_or(all_items.len());
                let items: Vec<_> = all_items.iter().skip(start).take(limit).cloned().collect();
                Ok(DatasetItemsPage { items })
            },
            count: |_name: &str, _version: u32| Ok(2),
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = AnnotationDataset::new(module, "ds", 1).unwrap();

        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 1 });
        assert_eq!(dataset.get(1).unwrap(), TestItem { value: 2 });
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    #[should_panic(expected = "Index out of bounds")]
    fn given_index_past_dataset_length_when_get_then_it_panics() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(1)],
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = AnnotationDataset::new(module, "ds", 1).unwrap();

        dataset.get(5).unwrap();
    }

    struct LatestFakeProvider<F, C, L> {
        stream: F,
        count: C,
        latest: L,
    }

    impl<F, C, L> DatasetProvider for LatestFakeProvider<F, C, L>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync,
        C: Fn(&str, u32) -> Result<u64, DatasetError> + Send + Sync,
        L: Fn(&str) -> Result<u32, DatasetError> + Send + Sync,
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

        fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
            (self.count)(name, version)
        }

        fn resolve_version(&self, name: &str) -> Result<u32, DatasetError> {
            (self.latest)(name)
        }
    }

    #[test]
    fn given_latest_spec_when_get_then_version_is_resolved_and_used() {
        let provider = LatestFakeProvider {
            stream: |_name: &str, version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                assert_eq!(version, 5);
                Ok(DatasetItemsPage {
                    items: vec![item(42)],
                })
            },
            count: |_name: &str, version: u32| {
                assert_eq!(version, 5);
                Ok(1)
            },
            latest: |_name: &str| Ok(5),
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module
            .as_burn_dataset("ds", DatasetVersionSpec::Latest)
            .unwrap();

        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 42 });
    }

    #[test]
    fn given_latest_spec_when_resolved_twice_then_version_is_resolved_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = LatestFakeProvider {
            stream: |_name: &str, _version: u32, _cursor: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(1)],
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
            latest: move |_name: &str| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(9)
            },
        };
        let module = DatasetModule::new(Arc::new(provider));
        let dataset: AnnotationDataset<TestItem> = module
            .as_burn_dataset("ds", DatasetVersionSpec::Latest)
            .unwrap();

        assert_eq!(dataset.len(), 1);
        assert_eq!(dataset.get(0).unwrap(), TestItem { value: 1 });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
