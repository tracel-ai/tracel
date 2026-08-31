use std::collections::HashSet;
use std::sync::Arc;

use serde::de::DeserializeOwned;

use crate::{
    Dataset, DatasetHandle, DatasetOps, DatasetVersion, DatasetsError, NewItem, Publication,
    VersionSpec,
};

/// Backend-independent dataset reading.
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
    pub fn list(&self) -> Result<Vec<Dataset>, DatasetsError> {
        self.ops.list_datasets()
    }

    /// Fetches one dataset by name.
    pub fn get(&self, name: &str) -> Result<Dataset, DatasetsError> {
        self.ops.get_dataset(name)
    }

    /// Lists a dataset's published versions.
    pub fn versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        self.ops.list_versions(dataset)
    }

    /// Resolves a version selector against a dataset.
    pub fn version(
        &self,
        dataset: &str,
        spec: impl Into<VersionSpec>,
    ) -> Result<DatasetVersion, DatasetsError> {
        self.ops.get_version(dataset, spec.into())
    }

    /// Opens a dataset at one version, resolving the selector once.
    ///
    /// `A` must match the annotation schema the dataset was published with.
    pub fn open<A>(
        &self,
        dataset: &str,
        spec: impl Into<VersionSpec>,
    ) -> Result<DatasetHandle<A>, DatasetsError>
    where
        A: DeserializeOwned,
    {
        let version = self.version(dataset, spec)?;
        Ok(DatasetHandle::new(self.ops.clone(), version))
    }

    /// Creates a dataset that can hold versions.
    pub fn create(
        &self,
        name: &str,
        description: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Dataset, DatasetsError> {
        self.ops.create_dataset(name, description, metadata)
    }

    /// Opens a draft that becomes a new version of `dataset` when committed.
    pub fn draft(&self, dataset: &str) -> Result<VersionDraft, DatasetsError> {
        Ok(VersionDraft {
            publication: self.ops.start_publication(dataset)?,
            offered: HashSet::new(),
            added: 0,
            settled: false,
        })
    }
}

/// A version being assembled, one item at a time.
///
/// Nothing is published until [`commit`](Self::commit); dropping the draft cancels it.
pub struct VersionDraft {
    publication: Box<dyn Publication>,
    offered: HashSet<String>,
    added: u64,
    settled: bool,
}

impl Drop for VersionDraft {
    fn drop(&mut self) {
        if !self.settled {
            let _ = self.publication.cancel();
        }
    }
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

        self.publication.add_item(item)?;
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

    /// Publishes everything added as a new version.
    ///
    /// A failed commit is not cancelled: the version may already exist.
    pub fn commit(
        mut self,
        metadata: Option<&serde_json::Value>,
    ) -> Result<DatasetVersion, DatasetsError> {
        self.settled = true;
        self.publication.commit(metadata)
    }

    /// Abandons the draft, discarding everything added.
    ///
    /// Dropping the draft does the same, without reporting failure.
    pub fn cancel(mut self) -> Result<(), DatasetsError> {
        self.settled = true;
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
        fn add_item(&mut self, _item: NewItem) -> Result<(), DatasetsError> {
            if self.failed {
                self.failed = false;
                return Err(DatasetsError::other("temporary failure"));
            }

            Ok(())
        }

        fn commit(
            &mut self,
            _metadata: Option<&serde_json::Value>,
        ) -> Result<DatasetVersion, DatasetsError> {
            unreachable!("this test only exercises adding")
        }

        fn cancel(&mut self) -> Result<(), DatasetsError> {
            Ok(())
        }
    }

    #[test]
    fn an_identity_is_not_reserved_when_adding_it_failed() {
        let mut draft = VersionDraft {
            publication: Box::new(FailOnce { failed: true }),
            offered: HashSet::new(),
            added: 0,
            settled: false,
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
