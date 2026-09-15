use std::marker::PhantomData;
use std::sync::Arc;

use futures::{TryStreamExt, stream};
use serde::de::DeserializeOwned;
use tracel_task::{Job, Streaming};

use crate::{DatasetOps, DatasetVersion, DatasetsError, Item};

/// Items one read asks for where the caller does not say.
const ITEMS_PER_READ: u64 = 64;

/// One item of a dataset version, with its annotation decoded into the caller's type.
#[derive(Clone, Debug, PartialEq)]
pub struct DatasetItem<A> {
    /// The example payload, as raw bytes.
    pub example: Vec<u8>,
    /// The annotation, when the item carries one.
    pub annotation: Option<A>,
    /// Identity of the source item, when the backend tracks it.
    pub source_item_id: Option<String>,
    /// Application-defined metadata.
    pub metadata: Option<serde_json::Value>,
}

/// One dataset at one version, ready to read.
///
/// Obtain one from [`Datasets::open`](crate::Datasets::open).
pub struct DatasetHandle<A> {
    ops: Arc<dyn DatasetOps>,
    version: DatasetVersion,
    annotation: PhantomData<A>,
}

impl<A> Clone for DatasetHandle<A> {
    fn clone(&self) -> Self {
        Self {
            ops: self.ops.clone(),
            version: self.version.clone(),
            annotation: PhantomData,
        }
    }
}

impl<A> DatasetHandle<A> {
    pub(crate) fn new(ops: Arc<dyn DatasetOps>, version: DatasetVersion) -> Self {
        Self {
            ops,
            version,
            annotation: PhantomData,
        }
    }

    /// The version being read.
    pub fn version(&self) -> &DatasetVersion {
        &self.version
    }

    /// How many items the version holds.
    pub fn len(&self) -> usize {
        self.version.item_count as usize
    }

    /// Whether the version holds no items.
    pub fn is_empty(&self) -> bool {
        self.version.item_count == 0
    }
}

impl<A> DatasetHandle<A>
where
    A: DeserializeOwned + Send + 'static,
{
    /// Reads the item at `index`.
    pub fn item(&self, index: u64) -> Job<DatasetItem<A>, DatasetsError> {
        let version = self.version.clone();
        let items = self.items(&[index]);
        Job::new(async move {
            items
                .await?
                .into_iter()
                .next()
                .ok_or(DatasetsError::Incomplete {
                    dataset: version.dataset,
                    version: version.id,
                    expected: 1,
                    actual: 0,
                })
        })
    }

    /// Reads the items at `indexes`, in the order asked for.
    ///
    /// Indexes need not be contiguous or sorted, so a shuffled batch is one call.
    pub fn items(&self, indexes: &[u64]) -> Job<Vec<DatasetItem<A>>, DatasetsError> {
        if let Some(past_end) = indexes
            .iter()
            .find(|index| **index >= self.version.item_count)
        {
            return Job::failed(DatasetsError::Item {
                dataset: self.version.dataset.clone(),
                version: self.version.id.clone(),
                index: *past_end,
                problem: format!("the version holds {} items", self.version.item_count),
            });
        }

        let version = self.version.clone();
        let indexes = indexes.to_vec();
        let read =
            self.ops
                .read_items(version.dataset.clone(), version.id.clone(), indexes.clone());
        Job::new(async move {
            let items = read.await?;
            if items.len() != indexes.len() {
                return Err(DatasetsError::Incomplete {
                    dataset: version.dataset,
                    version: version.id,
                    expected: indexes.len() as u64,
                    actual: items.len() as u64,
                });
            }

            items
                .into_iter()
                .zip(indexes)
                .map(|(item, index)| decode(&version, item, index))
                .collect()
        })
    }

    /// Streams the items from `from` to the end of the version, reading
    /// [`ITEMS_PER_READ`] at a time.
    pub fn iter(&self, from: u64) -> Streaming<DatasetItem<A>, DatasetsError> {
        self.iter_by(from, ITEMS_PER_READ)
    }

    /// Streams the items from `from` to the end of the version, reading `items_per_read` at a
    /// time.
    pub fn iter_by(
        &self,
        from: u64,
        items_per_read: u64,
    ) -> Streaming<DatasetItem<A>, DatasetsError> {
        let handle = self.clone();
        let count = self.version.item_count;
        let items_per_read = items_per_read.max(1);
        let batches = stream::try_unfold(from, move |next| {
            let handle = handle.clone();
            async move {
                if next >= count {
                    return Ok(None);
                }
                let end = (next + items_per_read).min(count);
                let indexes: Vec<u64> = (next..end).collect();
                let batch = handle.items(&indexes).await?;
                Ok(Some((stream::iter(batch.into_iter().map(Ok)), end)))
            }
        });

        Streaming::new(batches.try_flatten())
    }
}

fn decode<A: DeserializeOwned>(
    version: &DatasetVersion,
    item: Item,
    index: u64,
) -> Result<DatasetItem<A>, DatasetsError> {
    let Item {
        example,
        annotation,
        source_item_id,
        metadata,
    } = item;

    let annotation = match annotation {
        None => None,
        Some(value) => {
            Some(
                serde_json::from_value(value).map_err(|error| DatasetsError::Annotation {
                    dataset: version.dataset.clone(),
                    version: version.id.clone(),
                    index,
                    problem: error.to_string(),
                })?,
            )
        }
    };

    Ok(DatasetItem {
        example,
        annotation,
        source_item_id,
        metadata,
    })
}

// Burn's `Dataset` is a synchronous contract, so this adapter blocks on each read and exists
// only where a thread can.
#[cfg(all(feature = "burn", not(target_arch = "wasm32")))]
impl<A> burn::data::dataset::Dataset<DatasetItem<A>, DatasetsError> for DatasetHandle<A>
where
    A: DeserializeOwned + Send + Sync + 'static,
{
    fn get(&self, index: usize) -> Result<DatasetItem<A>, DatasetsError> {
        let len = self.len();
        assert!(
            index < len,
            "Index out of bounds for dataset: {index} >= {len}"
        );

        self.item(index as u64).block()
    }

    fn get_many(&self, indexes: Vec<usize>) -> Result<Vec<DatasetItem<A>>, DatasetsError> {
        let len = self.len();
        let indexes: Vec<u64> = indexes
            .into_iter()
            .map(|index| {
                assert!(
                    index < len,
                    "Index out of bounds for dataset: {index} >= {len}"
                );
                index as u64
            })
            .collect();

        self.items(&indexes).block()
    }

    fn len(&self) -> usize {
        self.version.item_count as usize
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::Datasets;
    use crate::VersionId;
    use crate::test_support::{FakeOps, Label, version};

    fn handle(ops: Arc<FakeOps>, item_count: u64) -> DatasetHandle<Label> {
        DatasetHandle::new(ops, version(item_count))
    }

    #[test]
    fn a_scattered_batch_is_one_read_in_the_order_asked_for() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops.clone(), 100);

        let items = data.items(&[92, 3, 17]).block().unwrap();

        assert_eq!(ops.reads(), 1);
        let examples: Vec<_> = items
            .iter()
            .map(|item| String::from_utf8(item.example.clone()).unwrap())
            .collect();
        assert_eq!(examples, ["92", "3", "17"]);
    }

    #[test]
    fn an_item_carries_its_provenance_and_metadata() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops, 10);

        let item = data.item(4).block().unwrap();

        assert_eq!(item.source_item_id.as_deref(), Some("source-4"));
        assert_eq!(item.metadata, Some(serde_json::json!({ "split": "train" })));
    }

    #[test]
    fn iterating_reads_in_batches_rather_than_one_request_per_item() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops.clone(), 600);

        let items: Vec<_> = data
            .iter(0)
            .blocking_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(items.len(), 600);
        assert_eq!(
            ops.reads(),
            10,
            "600 items over the default 64 a read asks for"
        );
    }

    #[test]
    fn a_caller_can_say_how_many_items_one_read_asks_for() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops.clone(), 600);

        let items: Vec<_> = data
            .iter_by(0, 256)
            .blocking_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(items.len(), 600);
        assert_eq!(ops.reads(), 3, "600 items over the 256 asked for");
    }

    /// A batch of zero would ask for no indexes, get none back, and leave the
    /// cursor where it was — the iterator would never end.
    #[test]
    fn a_read_of_zero_items_is_read_as_one_rather_than_never_advancing() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops.clone(), 5);

        let items: Vec<_> = data
            .iter_by(0, 0)
            .blocking_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(items.len(), 5);
        assert_eq!(ops.reads(), 5, "one item per read");
    }

    #[test]
    fn an_index_past_the_end_is_refused_before_the_backend_is_asked() {
        let ops = Arc::new(FakeOps::new());
        let data = handle(ops.clone(), 10);

        let error = data
            .items(&[3, 10])
            .block()
            .expect_err("index 10 is past the end of a 10 item version");

        assert!(matches!(error, DatasetsError::Item { index: 10, .. }));
        assert_eq!(ops.reads(), 0);
    }

    #[test]
    fn a_backend_that_answers_short_reports_an_incomplete_read() {
        let mut ops = FakeOps::new();
        ops.short_by = 1;
        let data = handle(Arc::new(ops), 100);

        let error = data
            .items(&[1, 2, 3])
            .block()
            .expect_err("the backend answered two of three indexes");

        assert!(matches!(
            error,
            DatasetsError::Incomplete {
                expected: 3,
                actual: 2,
                ..
            }
        ));
    }

    #[test]
    fn an_annotation_of_the_wrong_type_names_the_item_that_failed() {
        let mut ops = FakeOps::new();
        ops.annotation = serde_json::json!({ "value": "not a number" });
        let data = handle(Arc::new(ops), 10);

        let error = data
            .items(&[4])
            .block()
            .expect_err("the annotation does not match Label");

        match error {
            DatasetsError::Annotation { index, .. } => assert_eq!(index, 4),
            other => panic!("expected an annotation error, got {other:?}"),
        }
    }

    #[test]
    fn opening_resolves_the_version_once() {
        let ops = Arc::new(FakeOps::new());
        let datasets = Datasets::new(ops.clone());

        let data = datasets
            .open::<Label>("ds", VersionId::new("v1"))
            .block()
            .unwrap();

        assert_eq!(data.len(), 10);
        assert_eq!(data.version().id, VersionId::new("v1"));
    }

    #[cfg(feature = "burn")]
    mod burn_adapter {
        use burn::data::dataset::Dataset as BurnDataset;

        use super::*;

        #[test]
        fn a_burn_batch_is_one_read_rather_than_one_per_index() {
            let ops = Arc::new(FakeOps::new());
            let data = handle(ops.clone(), 100);

            let items = data.get_many(vec![92, 3, 17]).unwrap();

            assert_eq!(items.len(), 3);
            assert_eq!(ops.reads(), 1, "burn's default would read once per index");
        }

        #[test]
        #[should_panic(expected = "Index out of bounds for dataset: 10 >= 10")]
        fn an_out_of_bounds_index_panics_as_the_trait_requires() {
            let ops = Arc::new(FakeOps::new());
            let data = handle(ops, 10);

            let _ = data.get(10);
        }

        #[test]
        fn a_retrieval_failure_on_an_in_bounds_index_is_an_error_not_a_panic() {
            let mut ops = FakeOps::new();
            ops.short_by = 1;
            let data = handle(Arc::new(ops), 10);

            let error = data.get(3).expect_err("the backend answered nothing");

            assert!(matches!(error, DatasetsError::Incomplete { .. }));
        }
    }

    mod publication {
        use super::*;
        use crate::NewItem;

        fn item(id: &str) -> NewItem {
            NewItem {
                source_item_id: Some(id.to_string()),
                example: b"payload".to_vec(),
                annotation: None,
                metadata: None,
            }
        }

        #[test]
        fn committing_publishes_every_item_added() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            draft.add(item("b")).unwrap();
            assert_eq!(draft.len(), 2);
            draft.commit(None).block().unwrap();

            assert_eq!(ops.received().len(), 2);
            assert_eq!(ops.commits(), 1);
        }

        #[test]
        fn the_backend_decides_when_items_travel() {
            let mut ops = FakeOps::new();
            ops.flush_every = 100;
            let ops = Arc::new(ops);
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft
                .extend((0..250).map(|index| item(&index.to_string())))
                .unwrap();
            assert_eq!(
                ops.batch_count(),
                0,
                "adding queues uploads, it does not drive them"
            );

            draft.flush().block().unwrap();
            assert_eq!(
                ops.batch_count(),
                2,
                "the backend's own batch size decides, not the capability's"
            );

            draft.commit(None).block().unwrap();
            assert_eq!(ops.batch_count(), 3, "the remainder goes on commit");
            assert_eq!(ops.received().len(), 250);
        }

        #[test]
        fn a_dropped_draft_cancels_instead_of_publishing() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            drop(draft);

            assert_eq!(ops.commits(), 0);
            assert_eq!(ops.cancels(), 1);
        }

        #[test]
        fn cancelling_reports_what_the_backend_said() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            draft.cancel().block().unwrap();

            assert_eq!(ops.cancels(), 1);
            assert_eq!(ops.commits(), 0);
        }

        #[test]
        fn a_committed_draft_is_not_cancelled_when_it_drops() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            draft.commit(None).block().unwrap();

            assert_eq!(ops.commits(), 1);
            assert_eq!(ops.cancels(), 0);
        }

        #[test]
        fn a_failed_commit_is_not_cancelled_behind_the_caller() {
            let mut ops = FakeOps::new();
            ops.commit_fails = true;
            let ops = Arc::new(ops);
            let datasets = Datasets::new(ops.clone());

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            draft.commit(None).block().expect_err("the backend refused");

            assert_eq!(
                ops.cancels(),
                0,
                "the version may already exist; discarding it is the caller's call"
            );
        }

        #[test]
        fn items_without_an_identity_are_never_refused() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops.clone());

            let anonymous = || NewItem {
                source_item_id: None,
                example: b"payload".to_vec(),
                annotation: None,
                metadata: None,
            };

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(anonymous()).unwrap();
            draft.add(anonymous()).unwrap();
            draft.commit(None).block().unwrap();

            assert_eq!(ops.received().len(), 2);
        }

        #[test]
        fn a_repeated_source_item_id_is_refused_rather_than_quietly_collapsed() {
            let ops = Arc::new(FakeOps::new());
            let datasets = Datasets::new(ops);

            let mut draft = datasets.draft("ds").block().unwrap();
            draft.add(item("a")).unwrap();
            let error = draft
                .add(item("a"))
                .expect_err("the same identity was offered twice");

            match error {
                DatasetsError::DuplicateItem { source_item_id } => {
                    assert_eq!(source_item_id, "a")
                }
                other => panic!("expected a duplicate error, got {other:?}"),
            }
        }
    }
}
