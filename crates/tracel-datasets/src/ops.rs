use tracel_task::{Job, MaybeSend};

use crate::{Dataset, DatasetVersion, DatasetsError, Item, NewItem, VersionId, VersionSpec};

/// A version being assembled by a backend.
///
/// Either committed or cancelled; never both. Dropped without either, an implementation
/// abandons it as best it can on its own.
pub trait Publication: MaybeSend + 'static {
    /// Accepts one item.
    ///
    /// Implementations choose when to send what they are given: the item may only be kept, or
    /// it may complete a batch, in which case the batch's upload is handed back for the caller
    /// to drive before committing.
    fn add_item(&mut self, item: NewItem) -> Result<Option<Job<(), DatasetsError>>, DatasetsError>;

    /// Sends anything still held and publishes it as a new version.
    fn commit(
        self: Box<Self>,
        metadata: Option<serde_json::Value>,
    ) -> Job<DatasetVersion, DatasetsError>;

    /// Abandons the publication, discarding whatever was added.
    fn cancel(self: Box<Self>) -> Job<(), DatasetsError>;
}

/// Backend primitives required by the dataset capability.
///
/// An implementation is already scoped to one location, so it is never asked which one. Every
/// operation is handed back as a [`Job`] that any executor can drive: an implementation whose
/// transport needs a runtime attaches the work to the one it owns before handing it back.
///
/// Report a missing dataset or version as such; report everything else through
/// [`DatasetsError::other`], which keeps the implementation's own error type intact.
pub trait DatasetOps: Send + Sync + 'static {
    /// Lists datasets in the implementation's scope.
    fn list_datasets(&self) -> Job<Vec<Dataset>, DatasetsError>;

    /// Fetches one dataset by name.
    fn get_dataset(&self, name: String) -> Job<Dataset, DatasetsError>;

    /// Lists published versions of a dataset.
    fn list_versions(&self, dataset: String) -> Job<Vec<DatasetVersion>, DatasetsError>;

    /// Resolves a version selector against a dataset.
    fn get_version(&self, dataset: String, spec: VersionSpec)
    -> Job<DatasetVersion, DatasetsError>;

    /// Creates a dataset that can hold versions.
    fn create_dataset(
        &self,
        name: String,
        description: Option<String>,
        metadata: Option<serde_json::Value>,
    ) -> Job<Dataset, DatasetsError>;

    /// Opens a publication that becomes a new version of `dataset`.
    fn start_publication(&self, dataset: String) -> Job<Box<dyn Publication>, DatasetsError>;

    /// Reads the items at `indexes`, counted in published order from zero.
    ///
    /// Answer with one item per index, in the order asked for. Indexes need not be
    /// contiguous or sorted.
    fn read_items(
        &self,
        dataset: String,
        id: VersionId,
        indexes: Vec<u64>,
    ) -> Job<Vec<Item>, DatasetsError>;
}
