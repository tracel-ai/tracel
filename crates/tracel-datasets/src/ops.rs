use crate::{Dataset, DatasetVersion, DatasetsError, Item, NewItem, VersionId, VersionSpec};

/// A version being assembled by a backend.
///
/// Either committed or cancelled; never both, and never neither.
pub trait Publication {
    /// Accepts one item.
    ///
    /// Implementations choose when to send what they are given, so returning `Ok` does not
    /// mean the item has been sent.
    fn add_item(&mut self, item: NewItem) -> Result<(), DatasetsError>;

    /// Sends anything still held and publishes it as a new version.
    ///
    /// Called at most once.
    fn commit(
        &mut self,
        metadata: Option<&serde_json::Value>,
    ) -> Result<DatasetVersion, DatasetsError>;

    /// Abandons the publication, discarding whatever was added.
    ///
    /// Called at most once, in place of [`Self::commit`]. Also called from `Drop`, where the
    /// error is discarded.
    fn cancel(&mut self) -> Result<(), DatasetsError>;
}

/// Backend primitives required by the dataset capability.
///
/// An implementation is already scoped to one location, so it is never asked which one.
///
/// Report a missing dataset or version as such; report everything else through
/// [`DatasetsError::other`], which keeps the implementation's own error type intact.
pub trait DatasetOps: Send + Sync + 'static {
    /// Lists datasets in the implementation's scope.
    fn list_datasets(&self) -> Result<Vec<Dataset>, DatasetsError>;

    /// Fetches one dataset by name.
    fn get_dataset(&self, name: &str) -> Result<Dataset, DatasetsError>;

    /// Lists published versions of a dataset.
    fn list_versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError>;

    /// Resolves a version selector against a dataset.
    fn get_version(
        &self,
        dataset: &str,
        spec: VersionSpec,
    ) -> Result<DatasetVersion, DatasetsError>;

    /// Creates a dataset that can hold versions.
    fn create_dataset(
        &self,
        name: &str,
        description: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Dataset, DatasetsError>;

    /// Opens a publication that becomes a new version of `dataset`.
    fn start_publication(&self, dataset: &str) -> Result<Box<dyn Publication>, DatasetsError>;

    /// Reads the items at `indexes`, counted in published order from zero.
    ///
    /// Answer with one item per index, in the order asked for. Indexes need not be
    /// contiguous or sorted.
    fn read_items(
        &self,
        dataset: &str,
        id: &VersionId,
        indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError>;
}
