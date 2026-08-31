use std::sync::Arc;

use tracel_client::console::dataset::request::{
    AddDatasetVersionUploadItemsRequest, CompleteDatasetVersionUploadRequest, CreateDatasetRequest,
    DatasetVersionUploadItemRequest, QueryDatasetVersionsRequest, QueryDatasetsRequest,
};
use tracel_client::console::dataset::response::{DatasetResponse, DatasetVersionResponse};
use tracel_datasets::{
    Dataset, DatasetOps, DatasetVersion, DatasetsError, Item, NewItem, Publication, VersionId,
    VersionSpec,
};

use crate::ConsoleError;
use crate::console::{ConsoleInner, console_timestamp};

/// How many items an upload holds before sending a batch.
const BATCH: usize = 256;

#[derive(Clone)]
pub(crate) struct ConsoleDatasetOps {
    pub(crate) inner: Arc<ConsoleInner>,
    pub(crate) owner: String,
    pub(crate) project: String,
}

impl ConsoleDatasetOps {
    fn versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        let response = self
            .inner
            .client
            .query_dataset_versions(
                &self.owner,
                &self.project,
                dataset,
                QueryDatasetVersionsRequest::default(),
            )
            .map_err(console_failure)?;

        Ok(response
            .items
            .into_iter()
            .map(|version| version_from_wire(dataset, version))
            .collect())
    }
}

impl DatasetOps for ConsoleDatasetOps {
    fn list_datasets(&self) -> Result<Vec<Dataset>, DatasetsError> {
        let response = self
            .inner
            .client
            .query_datasets(&self.owner, &self.project, QueryDatasetsRequest::default())
            .map_err(console_failure)?;

        Ok(response.items.into_iter().map(dataset_from_wire).collect())
    }

    fn get_dataset(&self, name: &str) -> Result<Dataset, DatasetsError> {
        self.inner
            .client
            .get_dataset(&self.owner, &self.project, name)
            .map(dataset_from_wire)
            .map_err(console_failure)
    }

    fn list_versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        self.versions(dataset)
    }

    fn get_version(
        &self,
        dataset: &str,
        spec: VersionSpec,
    ) -> Result<DatasetVersion, DatasetsError> {
        let versions = self.versions(dataset)?;
        let found = match spec {
            VersionSpec::Fixed(wanted) => versions
                .into_iter()
                .find(|version| version.version == wanted),
            VersionSpec::Latest => versions.into_iter().max_by_key(|version| version.version),
        };

        found.ok_or(DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: spec,
        })
    }

    fn create_dataset(
        &self,
        name: &str,
        description: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Dataset, DatasetsError> {
        self.inner
            .client
            .create_dataset(
                &self.owner,
                &self.project,
                CreateDatasetRequest {
                    name: name.to_string(),
                    description: description.map(str::to_string),
                    metadata: metadata.cloned(),
                },
            )
            .map(dataset_from_wire)
            .map_err(console_failure)
    }

    fn start_publication(&self, dataset: &str) -> Result<Box<dyn Publication>, DatasetsError> {
        let started = self
            .inner
            .client
            .start_dataset_version_upload(&self.owner, &self.project, dataset)
            .map_err(console_failure)?;

        Ok(Box::new(ConsolePublication {
            ops: self.clone(),
            dataset: dataset.to_string(),
            upload_id: started.upload_id,
            pending: Vec::new(),
        }))
    }

    fn read_items(
        &self,
        _dataset: &str,
        _id: &VersionId,
        _indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError> {
        Err(DatasetsError::other(
            "reading dataset items is not implemented for the console yet",
        ))
    }
}

/// One upload, sending items in batches.
struct ConsolePublication {
    ops: ConsoleDatasetOps,
    dataset: String,
    upload_id: String,
    pending: Vec<DatasetVersionUploadItemRequest>,
}

impl ConsolePublication {
    fn flush(&mut self) -> Result<(), DatasetsError> {
        if self.pending.is_empty() {
            return Ok(());
        }

        self.ops
            .inner
            .client
            .add_dataset_version_upload_items(
                &self.ops.owner,
                &self.ops.project,
                &self.dataset,
                &self.upload_id,
                AddDatasetVersionUploadItemsRequest {
                    items: std::mem::take(&mut self.pending),
                },
            )
            .map(|_| ())
            .map_err(console_failure)
    }
}

impl Publication for ConsolePublication {
    fn add_item(&mut self, item: NewItem) -> Result<(), DatasetsError> {
        self.pending.push(DatasetVersionUploadItemRequest {
            source_item_id: item.source_item_id,
            example_payload: item.example,
            annotation: item.annotation,
            metadata: item.metadata,
        });

        if self.pending.len() >= BATCH {
            self.flush()?;
        }

        Ok(())
    }

    fn commit(
        &mut self,
        metadata: Option<&serde_json::Value>,
    ) -> Result<DatasetVersion, DatasetsError> {
        self.flush()?;

        self.ops
            .inner
            .client
            .complete_dataset_version_upload(
                &self.ops.owner,
                &self.ops.project,
                &self.dataset,
                &self.upload_id,
                CompleteDatasetVersionUploadRequest {
                    metadata: metadata.cloned(),
                },
            )
            .map(|version| version_from_wire(&self.dataset, version))
            .map_err(console_failure)
    }

    fn cancel(&mut self) -> Result<(), DatasetsError> {
        self.pending.clear();
        self.ops
            .inner
            .client
            .cancel_dataset_version_upload(
                &self.ops.owner,
                &self.ops.project,
                &self.dataset,
                &self.upload_id,
            )
            .map_err(console_failure)
    }
}

fn dataset_from_wire(response: DatasetResponse) -> Dataset {
    Dataset {
        name: response.name,
        description: response.description,
        metadata: response.metadata,
    }
}

fn version_from_wire(dataset: &str, response: DatasetVersionResponse) -> DatasetVersion {
    DatasetVersion {
        dataset: dataset.to_string(),
        id: VersionId::new(response.id),
        version: response.version.max(0) as u32,
        item_count: response.item_count,
        metadata: response.metadata,
        created_at: console_timestamp(&response.created_at),
    }
}

fn console_failure(error: tracel_client::ClientError) -> DatasetsError {
    DatasetsError::other(ConsoleError::from(error))
}
