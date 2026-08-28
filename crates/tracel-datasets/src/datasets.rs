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
    /// Fails if the draft already holds an item with the same source identity.
    pub fn add(&mut self, item: NewItem) -> Result<(), DatasetsError> {
        if !self.offered.insert(item.source_item_id.clone()) {
            return Err(DatasetsError::DuplicateItem {
                source_item_id: item.source_item_id,
            });
        }

        self.publication.add_item(item)?;
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
