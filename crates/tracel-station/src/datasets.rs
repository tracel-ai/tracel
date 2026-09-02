use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use tracel_client::station::dataset::request::{
    CreateDatasetRequest, QueryDatasetVersionsRequest, QueryDatasetsRequest,
    StreamDatasetVersionItemsRequest,
};
use tracel_client::station::dataset::response::{DatasetResponse, DatasetVersionResponse};
use tracel_datasets::{
    Dataset, DatasetOps, DatasetVersion, DatasetsError, Item, Publication, VersionId, VersionSpec,
};

use crate::StationError;
use crate::station::StationInner;
use crate::wire::station_timestamp;

pub struct StationDatasetOps {
    pub station: Arc<StationInner>,
}

impl StationDatasetOps {
    fn route_version(&self, dataset: &str, id: &VersionId) -> Result<u32, DatasetsError> {
        id.as_str()
            .parse()
            .map_err(|_| DatasetsError::VersionNotFound {
                dataset: dataset.to_string(),
                version: VersionSpec::Exact(id.clone()),
            })
    }
}

impl DatasetOps for StationDatasetOps {
    fn list_datasets(&self) -> Result<Vec<Dataset>, DatasetsError> {
        let response = self
            .station
            .client
            .datasets()
            .query(QueryDatasetsRequest::default())
            .map_err(station_failure)?;
        Ok(response.items.into_iter().map(dataset_from_wire).collect())
    }

    /// The Station has no route for one dataset by name, so this reads the listing.
    fn get_dataset(&self, name: &str) -> Result<Dataset, DatasetsError> {
        self.list_datasets()?
            .into_iter()
            .find(|dataset| dataset.name == name)
            .ok_or_else(|| DatasetsError::DatasetNotFound {
                name: name.to_string(),
            })
    }

    fn list_versions(&self, dataset: &str) -> Result<Vec<DatasetVersion>, DatasetsError> {
        let response = self
            .station
            .client
            .datasets()
            .versions(dataset, QueryDatasetVersionsRequest::default())
            .map_err(|error| map_dataset_error(error, dataset))?;

        response
            .items
            .into_iter()
            .map(|version| version_from_wire(dataset, version))
            .collect()
    }

    fn get_version(
        &self,
        dataset: &str,
        spec: VersionSpec,
    ) -> Result<DatasetVersion, DatasetsError> {
        let versions = self.station.client.datasets();
        let response = match &spec {
            VersionSpec::Exact(id) => {
                versions.get_version(dataset, self.route_version(dataset, id)?)
            }
            VersionSpec::Latest => versions.get_latest_version(dataset),
        };

        response
            .map_err(|error| {
                if error.is_not_found() {
                    DatasetsError::VersionNotFound {
                        dataset: dataset.to_string(),
                        version: spec.clone(),
                    }
                } else {
                    station_failure(error)
                }
            })
            .and_then(|version| version_from_wire(dataset, version))
    }

    fn create_dataset(
        &self,
        name: &str,
        description: Option<&str>,
        metadata: Option<&serde_json::Value>,
    ) -> Result<Dataset, DatasetsError> {
        self.station
            .client
            .datasets()
            .create(CreateDatasetRequest {
                name: name.to_string(),
                description: description.map(str::to_string),
                metadata: metadata.cloned(),
            })
            .map(dataset_from_wire)
            .map_err(station_failure)
    }

    fn start_publication(&self, _dataset: &str) -> Result<Box<dyn Publication>, DatasetsError> {
        Err(DatasetsError::other(
            "publishing a dataset version is not implemented for the station yet",
        ))
    }

    fn read_items(
        &self,
        dataset: &str,
        id: &VersionId,
        indexes: &[u64],
    ) -> Result<Vec<Item>, DatasetsError> {
        let version = self.route_version(dataset, id)?;
        let mut read = HashMap::with_capacity(indexes.len());

        for run in contiguous_runs(indexes) {
            let mut next = run.start;
            while next < run.end {
                let page = self
                    .station
                    .client
                    .datasets()
                    .stream_items(
                        dataset,
                        version,
                        StreamDatasetVersionItemsRequest {
                            index: Some(next),
                            limit: Some((run.end - next).min(u32::MAX as u64) as u32),
                        },
                    )
                    .map_err(|error| map_version_error(error, dataset, id))?;

                if page.items.is_empty() {
                    break;
                }

                let asked_from = next;
                for item in page.items {
                    next = item.entry_idx + 1;
                    if item.entry_idx < run.end {
                        read.insert(item.entry_idx, item_from_wire(&item.payload)?);
                    }
                }

                // A page that leaves the cursor where it was would be asked for forever.
                if next <= asked_from {
                    break;
                }
            }
        }

        let found = read.len() as u64;
        ordered_items(indexes, &read).ok_or(DatasetsError::Incomplete {
            dataset: dataset.to_string(),
            version: id.clone(),
            expected: indexes.len() as u64,
            actual: found,
        })
    }
}

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

fn ordered_items(indexes: &[u64], items: &HashMap<u64, Item>) -> Option<Vec<Item>> {
    indexes
        .iter()
        .map(|index| items.get(index).cloned())
        .collect()
}

fn item_from_wire(payload: &[u8]) -> Result<Item, DatasetsError> {
    let wire: WireItem = serde_json::from_slice(payload)
        .map_err(|error| DatasetsError::other(StationError::InvalidResponse(error.to_string())))?;

    Ok(Item {
        example: wire.example_payload,
        annotation: wire.annotation,
        source_item_id: wire.source_item_id,
        metadata: wire.metadata,
    })
}

fn dataset_from_wire(response: DatasetResponse) -> Dataset {
    Dataset {
        name: response.name,
        description: response.description,
        metadata: response.metadata,
    }
}

fn version_from_wire(
    dataset: &str,
    response: DatasetVersionResponse,
) -> Result<DatasetVersion, DatasetsError> {
    let version = u32::try_from(response.version).map_err(|_| {
        DatasetsError::other(StationError::InvalidResponse(format!(
            "dataset '{dataset}' reported version {}",
            response.version
        )))
    })?;

    Ok(DatasetVersion {
        dataset: dataset.to_string(),
        id: VersionId::new(version.to_string()),
        version: Some(version),
        item_count: response.item_count,
        metadata: response.metadata,
        created_at: station_timestamp(&response.created_at),
    })
}

fn station_failure(error: tracel_client::ClientError) -> DatasetsError {
    DatasetsError::other(StationError::from(error))
}

fn map_dataset_error(error: tracel_client::ClientError, dataset: &str) -> DatasetsError {
    if error.is_not_found() {
        return DatasetsError::DatasetNotFound {
            name: dataset.to_string(),
        };
    }
    station_failure(error)
}

fn map_version_error(
    error: tracel_client::ClientError,
    dataset: &str,
    id: &VersionId,
) -> DatasetsError {
    if error.is_not_found() {
        return DatasetsError::VersionNotFound {
            dataset: dataset.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    station_failure(error)
}

#[cfg(test)]
mod tests {
    use tracel_client::station::dataset::response::SourceKindResponse;

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

    #[test]
    fn a_repeated_index_is_returned_each_time() {
        let item = Item {
            example: b"three".to_vec(),
            annotation: None,
            source_item_id: None,
            metadata: None,
        };
        let items = HashMap::from([(3, item.clone())]);

        assert_eq!(
            ordered_items(&[3, 3], &items),
            Some(vec![item.clone(), item])
        );
    }

    #[test]
    fn a_negative_version_is_a_broken_response() {
        let response = DatasetVersionResponse {
            id: "1".to_string(),
            dataset_id: "1".to_string(),
            version: -1,
            metadata: None,
            source_kind: SourceKindResponse::AnnotationSet,
            created_at: "2026-09-01T00:00:00Z".to_string(),
            item_count: 0,
        };

        let error = version_from_wire("mnist", response).expect_err("a version cannot be negative");
        assert!(!error.is_not_found());
    }
}
