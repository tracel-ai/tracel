use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use crate::models::client_error_is_not_found;
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
use crate::console::ProjectScope;
use crate::wire::console_timestamp;

/// How many items an upload holds before sending a batch.
const BATCH: usize = 256;

#[derive(Clone)]
pub struct ConsoleDatasetOps {
    pub scope: Arc<ProjectScope>,
}

impl ConsoleDatasetOps {
    fn versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        let response = self
            .scope
            .console
            .client
            .query_dataset_versions(
                &self.scope.owner,
                &self.scope.project,
                dataset,
                QueryDatasetVersionsRequest::default(),
            )
            .map_err(|error| map_dataset_error(error, dataset))?;

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
            .scope
            .console
            .client
            .query_datasets(
                &self.scope.owner,
                &self.scope.project,
                QueryDatasetsRequest::default(),
            )
            .map_err(console_failure)?;

        Ok(response.items.into_iter().map(dataset_from_wire).collect())
    }

    fn get_dataset(&self, name: &str) -> Result<Dataset, DatasetsError> {
        self.scope
            .console
            .client
            .get_dataset(&self.scope.owner, &self.scope.project, name)
            .map(dataset_from_wire)
            .map_err(|error| map_dataset_error(error, name))
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
        let found = match &spec {
            VersionSpec::Exact(wanted) => {
                versions.into_iter().find(|version| &version.id == wanted)
            }
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
        self.scope
            .console
            .client
            .create_dataset(
                &self.scope.owner,
                &self.scope.project,
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
            .scope
            .console
            .client
            .start_dataset_version_upload(&self.scope.owner, &self.scope.project, dataset)
            .map_err(|error| map_dataset_error(error, dataset))?;

        Ok(Box::new(ConsolePublication {
            ops: self.clone(),
            dataset: dataset.to_string(),
            upload_id: started.upload_id,
            pending: Vec::new(),
        }))
    }

    fn read_items(
        &self,
        dataset: &str,
        id: &VersionId,
        indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError> {
        let version = route_version(dataset, id)?;
        let mut read = HashMap::with_capacity(indexes.len());

        for run in contiguous_runs(indexes) {
            let mut next = run.start;
            while next < run.end {
                let page = self
                    .scope
                    .console
                    .client
                    .stream_dataset_version_items(
                        &self.scope.owner,
                        &self.scope.project,
                        dataset,
                        version,
                        Some(next),
                        Some((run.end - next).min(u32::MAX as u64) as u32),
                    )
                    .map_err(|error| map_version_error(error, dataset, id))?;

                if page.items.is_empty() {
                    break;
                }

                for item in page.items {
                    next = item.entry_idx + 1;
                    if item.entry_idx < run.end {
                        read.insert(item.entry_idx, item_from_wire(&item.payload)?);
                    }
                }
            }
        }

        let found = read.len() as u64;
        indexes
            .iter()
            .map(|index| read.remove(index))
            .collect::<Option<Vec<_>>>()
            .ok_or(DatasetsError::Incomplete {
                dataset: dataset.to_string(),
                version: id.clone(),
                expected: indexes.len() as u64,
                actual: found,
            })
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
            .scope
            .console
            .client
            .add_dataset_version_upload_items(
                &self.ops.scope.owner,
                &self.ops.scope.project,
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
            .scope
            .console
            .client
            .complete_dataset_version_upload(
                &self.ops.scope.owner,
                &self.ops.scope.project,
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
            .scope
            .console
            .client
            .cancel_dataset_version_upload(
                &self.ops.scope.owner,
                &self.ops.scope.project,
                &self.dataset,
                &self.upload_id,
            )
            .map_err(console_failure)
    }
}

/// The document a streamed item carries, shared by every backend the console serves.
#[serde_with::serde_as]
#[derive(serde::Deserialize)]
struct WireItem {
    source_item_id: Option<String>,
    metadata: Option<serde_json::Value>,
    #[serde_as(as = "serde_with::base64::Base64")]
    example_payload: Vec<u8>,
    annotation: Option<serde_json::Value>,
}

/// The ascending, contiguous stretches `indexes` covers, so each is one request.
fn contiguous_runs(indexes: &[u64]) -> Vec<Range<u64>> {
    let mut sorted: Vec<u64> = indexes.to_vec();
    sorted.sort_unstable();
    sorted.dedup();

    let mut runs: Vec<Range<u64>> = Vec::new();
    for index in sorted {
        match runs.last_mut() {
            Some(run) if run.end == index => run.end = index + 1,
            _ => runs.push(index..index + 1),
        }
    }

    runs
}

fn item_from_wire(payload: &[u8]) -> Result<Item, DatasetsError> {
    let wire: WireItem = serde_json::from_slice(payload)
        .map_err(|error| DatasetsError::other(ConsoleError::InvalidResponse(error.to_string())))?;

    Ok(Item {
        example: wire.example_payload,
        annotation: wire.annotation,
        source_item_id: wire.source_item_id,
        metadata: wire.metadata,
    })
}

fn route_version(dataset: &str, id: &VersionId) -> Result<u32, DatasetsError> {
    id.as_str()
        .parse()
        .map_err(|_| DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: VersionSpec::Exact(id.clone()),
        })
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
        id: VersionId::new(response.version.max(0).to_string()),
        version: Some(response.version.max(0) as u32),
        item_count: response.item_count,
        metadata: response.metadata,
        created_at: console_timestamp(&response.created_at),
    }
}

fn console_failure(error: tracel_client::ClientError) -> DatasetsError {
    DatasetsError::other(ConsoleError::from(error))
}

/// Reads a client failure as the dataset problem it stands for.
fn map_dataset_error(error: tracel_client::ClientError, dataset: &str) -> DatasetsError {
    if client_error_is_not_found(&error) {
        return DatasetsError::DatasetNotFound {
            name: dataset.to_string(),
        };
    }
    console_failure(error)
}

fn map_version_error(
    error: tracel_client::ClientError,
    dataset: &str,
    id: &VersionId,
) -> DatasetsError {
    if client_error_is_not_found(&error) {
        return DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    console_failure(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_batch_of_neighbours_is_one_run() {
        assert_eq!(contiguous_runs(&[4, 5, 6]), vec![4..7]);
    }

    #[test]
    fn a_shuffled_batch_is_sorted_and_coalesced() {
        assert_eq!(contiguous_runs(&[9, 1, 8, 0, 2]), vec![0..3, 8..10]);
    }

    #[test]
    fn a_repeated_index_is_asked_for_once() {
        assert_eq!(contiguous_runs(&[3, 3, 3]), vec![3..4]);
    }
}
