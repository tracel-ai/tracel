use burn::data::dataset::{Dataset, DatasetError as BurnDatasetError};
use serde::de::DeserializeOwned;

use super::item::{ItemLocation, decode_item};
use super::{DatasetError, DatasetItem, DatasetSource};

/// A Station dataset version whose items are fetched from the backend on demand.
///
/// Each call to [`Dataset::get`] requests one envelope from the backend, decodes its example
/// payload into raw bytes, and deserializes its optional annotation into `A`. The item count is
/// resolved once at construction and cached for [`Dataset::len`]. See
/// [`DownloadedDataset`](super::DownloadedDataset) for an adapter that downloads all raw item
/// envelopes up front.
///
/// `get` reports failures as Burn's [`BurnDatasetError`]. The original [`DatasetError`] is
/// preserved as the source and can be recovered with [`std::error::Error::source`] and a
/// downcast.
///
/// Build one with [`DatasetModule::stream`](super::DatasetModule::stream) rather than
/// constructing it directly.
pub struct StreamedDataset<A> {
    source: DatasetSource,
    name: String,
    version: u32,
    len: usize,
    _marker: std::marker::PhantomData<A>,
}

impl<A> StreamedDataset<A> {
    pub(super) fn new(
        source: DatasetSource,
        name: String,
        version: u32,
    ) -> Result<Self, DatasetError> {
        let len = source.item_count(&name, version)? as usize;

        Ok(Self {
            source,
            name,
            version,
            len,
            _marker: std::marker::PhantomData,
        })
    }
}

impl<A> Dataset<DatasetItem<A>> for StreamedDataset<A>
where
    A: DeserializeOwned + Clone + Send + Sync,
{
    fn get(&self, index: usize) -> Result<DatasetItem<A>, BurnDatasetError> {
        let len = self.len();
        assert!(
            index < len,
            "Index out of bounds for StreamedDataset: {index} >= {len}"
        );

        let page = self
            .source
            .get_items(&self.name, self.version, Some(index as u64), Some(1))
            .map_err(BurnDatasetError::new)?;

        let raw_item = page.items.into_iter().next().ok_or_else(|| {
            BurnDatasetError::new(DatasetError::MissingItem {
                name: self.name.clone(),
                version: self.version,
                index: index as u64,
            })
        })?;

        decode_item(
            &raw_item,
            &self.name,
            self.version,
            index as u64,
            ItemLocation::Streamed,
        )
        .map_err(BurnDatasetError::new)
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use burn::data::dataset::Dataset;
    use serde::Deserialize;

    use super::StreamedDataset;
    use crate::dataset::item::envelope_item;
    use crate::dataset::{DatasetError, DatasetItemsPage, DatasetSource};

    #[derive(Debug, Clone, Deserialize, PartialEq)]
    struct TestAnnotation {
        value: u32,
    }

    struct FakeProvider<F, C> {
        stream: F,
        count: C,
    }

    impl<F, C> FakeProvider<F, C>
    where
        F: Fn(&str, u32, Option<u64>, Option<u32>) -> Result<DatasetItemsPage, DatasetError>
            + Send
            + Sync
            + 'static,
        C: Fn(&str, u32) -> Result<u64, DatasetError> + Send + Sync + 'static,
    {
        fn into_source(self) -> DatasetSource {
            DatasetSource::new(self.stream, self.count, |_name| {
                unreachable!("StreamedDataset only receives resolved versions")
            })
        }
    }

    fn item(value: u32) -> Vec<u8> {
        envelope_item(
            format!("example-{value}").as_bytes(),
            Some(serde_json::json!({ "value": value })),
        )
    }

    #[test]
    fn given_valid_envelope_when_get_then_dataset_item_is_decoded() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(42)],
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 1)
                .unwrap();

        let item = dataset.get(0).unwrap();
        assert_eq!(item.example, b"example-42");
        assert_eq!(item.annotation, Some(TestAnnotation { value: 42 }));
    }

    #[test]
    fn given_malformed_envelope_when_get_then_error_is_returned_and_valid_item_still_decodes() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![match index {
                        Some(1) => item(7),
                        _ => b"not json".to_vec(),
                    }],
                })
            },
            count: |_name: &str, _version: u32| Ok(2),
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 1)
                .unwrap();

        assert_eq!(dataset.len(), 2);
        assert!(dataset.get(0).is_err());
        assert_eq!(
            dataset.get(1).unwrap().annotation,
            Some(TestAnnotation { value: 7 })
        );
    }

    #[test]
    fn given_dataset_when_len_called_twice_then_item_count_is_fetched_only_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage { items: vec![] })
            },
            count: move |_name: &str, _version: u32| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                Ok(2)
            },
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 1)
                .unwrap();

        assert_eq!(dataset.len(), 2);
        assert_eq!(dataset.len(), 2);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn given_two_indices_when_get_called_twice_then_provider_receives_each_index() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let all_items = [item(1), item(2)];
        let provider = FakeProvider {
            stream: move |_name: &str, _version: u32, index: Option<u64>, limit: Option<u32>| {
                calls_clone.fetch_add(1, Ordering::SeqCst);
                let start = index.unwrap_or(0) as usize;
                let limit = limit.map(|value| value as usize).unwrap_or(all_items.len());
                let items = all_items.iter().skip(start).take(limit).cloned().collect();
                Ok(DatasetItemsPage { items })
            },
            count: |_name: &str, _version: u32| Ok(2),
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 1)
                .unwrap();

        assert_eq!(
            dataset.get(0).unwrap().annotation,
            Some(TestAnnotation { value: 1 })
        );
        assert_eq!(
            dataset.get(1).unwrap().annotation,
            Some(TestAnnotation { value: 2 })
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn given_provider_returns_no_item_when_get_then_missing_item_error_is_returned() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage { items: vec![] })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 3)
                .unwrap();

        let err = dataset.get(0).unwrap_err();
        let source = err.source().unwrap();

        assert!(matches!(
            source.downcast_ref::<DatasetError>(),
            Some(DatasetError::MissingItem {
                name,
                version: 3,
                index: 0,
            }) if name == "ds"
        ));
    }

    #[test]
    #[should_panic(expected = "Index out of bounds")]
    fn given_index_past_dataset_length_when_get_then_it_panics() {
        let provider = FakeProvider {
            stream: |_name: &str, _version: u32, _index: Option<u64>, _limit: Option<u32>| {
                Ok(DatasetItemsPage {
                    items: vec![item(1)],
                })
            },
            count: |_name: &str, _version: u32| Ok(1),
        };
        let dataset =
            StreamedDataset::<TestAnnotation>::new(provider.into_source(), "ds".to_string(), 1)
                .unwrap();

        dataset.get(5).unwrap();
    }
}
