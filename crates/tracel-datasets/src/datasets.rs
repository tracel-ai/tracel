use std::collections::HashSet;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use tracel_task::Job;

use crate::{
    Dataset, DatasetHandle, DatasetOps, DatasetVersion, DatasetsError, NewItem, Publication,
    VersionSpec,
};

/// Backend-independent dataset reading and publishing.
///
/// Every operation is handed back as a [`Job`]: await it, block on it at a native edge, poll it
/// per tick, or spawn it wherever the caller runs things. Nothing runs until the job is driven.
#[derive(Clone)]
pub struct Datasets {
    ops: Arc<dyn DatasetOps>,
}

impl Datasets {
    /// Builds the capability over a backend's primitives.
    pub fn new(ops: Arc<dyn DatasetOps>) -> Self {
        Self { ops }
    }

    /// Lists the datasets in scope.
    pub fn list(&self) -> Job<Vec<Dataset>, DatasetsError> {
        self.ops.list_datasets()
    }

    /// Fetches one dataset by name.
    pub fn get(&self, name: impl Into<String>) -> Job<Dataset, DatasetsError> {
        self.ops.get_dataset(name.into())
    }

    /// Lists a dataset's published versions.
    pub fn versions(&self, dataset: impl Into<String>) -> Job<Vec<DatasetVersion>, DatasetsError> {
        self.ops.list_versions(dataset.into())
    }

    /// Resolves a version selector against a dataset.
    pub fn version(
        &self,
        dataset: impl Into<String>,
        spec: impl Into<VersionSpec>,
    ) -> Job<DatasetVersion, DatasetsError> {
        self.ops.get_version(dataset.into(), spec.into())
    }

    /// Opens a dataset at one version, resolving the selector once.
    ///
    /// `A` must match the annotation schema the dataset was published with.
    pub fn open<A>(
        &self,
        dataset: impl Into<String>,
        spec: impl Into<VersionSpec>,
    ) -> Job<DatasetHandle<A>, DatasetsError>
    where
        A: DeserializeOwned + Send + 'static,
    {
        let ops = Arc::clone(&self.ops);
        let version = self.version(dataset, spec);
        Job::new(async move { Ok(DatasetHandle::new(ops, version.await?)) })
    }

    /// Creates a dataset that can hold versions.
    pub fn create(
        &self,
        name: impl Into<String>,
        description: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Job<Dataset, DatasetsError> {
        self.ops.create_dataset(name.into(), description, metadata)
    }

    /// Opens a draft that becomes a new version of `dataset` when committed.
    pub fn draft(&self, dataset: impl Into<String>) -> Job<VersionDraft, DatasetsError> {
        let publication = self.ops.start_publication(dataset.into());
        Job::new(async move {
            Ok(VersionDraft {
                publication: publication.await?,
                offered: HashSet::new(),
                added: 0,
                uploads: None,
            })
        })
    }
}

/// A version being assembled, one item at a time.
///
/// Adding never waits: an item is kept, or completes a batch whose upload is queued behind the
/// previous ones. [`flush`](Self::flush) hands the queued uploads back to drive;
/// [`commit`](Self::commit) drives them before publishing. Nothing is published until then, and
/// dropping the draft abandons it.
pub struct VersionDraft {
    publication: Box<dyn Publication>,
    offered: HashSet<String>,
    added: u64,
    uploads: Option<Job<(), DatasetsError>>,
}

impl VersionDraft {
    /// Adds one item.
    ///
    /// Fails if the draft already holds an item claiming the same source identity. Items
    /// offered without one are never refused.
    pub fn add(&mut self, item: NewItem) -> Result<(), DatasetsError> {
        let identity = item.source_item_id.clone();
        if let Some(identity) = &identity {
            if self.offered.contains(identity) {
                return Err(DatasetsError::DuplicateItem {
                    source_item_id: identity.clone(),
                });
            }
        }

        if let Some(upload) = self.publication.add_item(item)? {
            self.uploads = Some(match self.uploads.take() {
                Some(previous) => Job::new(async move {
                    previous.await?;
                    upload.await
                }),
                None => upload,
            });
        }
        if let Some(identity) = identity {
            self.offered.insert(identity);
        }
        self.added += 1;
        Ok(())
    }

    /// Adds every item, stopping at the first one refused.
    pub fn extend(
        &mut self,
        items: impl IntoIterator<Item = NewItem>,
    ) -> Result<(), DatasetsError> {
        for item in items {
            self.add(item)?;
        }

        Ok(())
    }

    /// How many items have been added so far.
    pub fn len(&self) -> u64 {
        self.added
    }

    /// Whether nothing has been added yet.
    pub fn is_empty(&self) -> bool {
        self.added == 0
    }

    /// Hands back the batch uploads queued so far, in order, so memory is released before the
    /// draft is committed. Ready at once when nothing is queued.
    pub fn flush(&mut self) -> Job<(), DatasetsError> {
        self.uploads.take().unwrap_or_else(|| Job::ready(()))
    }

    /// Publishes everything added as a new version, driving any queued uploads first.
    ///
    /// A failed commit is not cancelled: the version may already exist.
    pub fn commit(
        mut self,
        metadata: Option<serde_json::Value>,
    ) -> Job<DatasetVersion, DatasetsError> {
        let uploads = self.flush();
        let publication = self.publication;
        Job::new(async move {
            uploads.await?;
            publication.commit(metadata).await
        })
    }

    /// Abandons the draft, discarding everything added.
    ///
    /// Dropping the draft does the same, without reporting failure.
    pub fn cancel(self) -> Job<(), DatasetsError> {
        self.publication.cancel()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailOnce {
        failed: bool,
    }

    impl Publication for FailOnce {
        fn add_item(
            &mut self,
            _item: NewItem,
        ) -> Result<Option<Job<(), DatasetsError>>, DatasetsError> {
            if self.failed {
                self.failed = false;
                return Err(DatasetsError::other("temporary failure"));
            }

            Ok(None)
        }

        fn commit(
            self: Box<Self>,
            _metadata: Option<serde_json::Value>,
        ) -> Job<DatasetVersion, DatasetsError> {
            unreachable!("this test only exercises adding")
        }

        fn cancel(self: Box<Self>) -> Job<(), DatasetsError> {
            Job::ready(())
        }
    }

    #[test]
    fn an_identity_is_not_reserved_when_adding_it_failed() {
        let mut draft = VersionDraft {
            publication: Box::new(FailOnce { failed: true }),
            offered: HashSet::new(),
            added: 0,
            uploads: None,
        };
        let item = NewItem {
            source_item_id: Some("a".to_string()),
            example: Vec::new(),
            annotation: None,
            metadata: None,
        };

        draft.add(item.clone()).expect_err("the first add fails");
        draft.add(item).expect("the same item can be retried");

        assert_eq!(draft.len(), 1);
    }
}
