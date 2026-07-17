use tracel_client::ClientError;
use tracel_client::station::dataset::StreamDatasetVersionItemsRequest;

use crate::backend::station::StationBackend;
use crate::dataset::{DatasetError, DatasetItemsPage, DatasetProvider, RawDatasetItem};

const QUERY_PAGE_SIZE: u32 = 100;

impl DatasetProvider for StationBackend {
    fn stream_items(
        &self,
        name: &str,
        version: u32,
        cursor: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError> {
        let response = self
            .client
            .datasets()
            .stream_items(
                name,
                version,
                StreamDatasetVersionItemsRequest { cursor, limit },
            )
            .map_err(|err| self.describe_stream_error(err, name, version))?;

        Ok(DatasetItemsPage {
            items: response
                .items
                .into_iter()
                .map(|item| RawDatasetItem {
                    entry_idx: item.entry_idx,
                    payload: item.payload,
                })
                .collect(),
            next_cursor: response.next_cursor,
        })
    }
}

impl StationBackend {
    fn describe_stream_error(&self, err: ClientError, name: &str, version: u32) -> DatasetError {
        if !matches!(err, ClientError::NotFound) {
            return DatasetError::Client(Box::new(err));
        }
        if let Err(e) = self.ensure_dataset_exists(name) {
            return e;
        }
        self.ensure_dataset_version_exists(name, version)
            .err()
            .unwrap_or(DatasetError::VersionNotFound {
                name: name.to_string(),
                version,
            })
    }

    fn ensure_dataset_exists(&self, name: &str) -> Result<(), DatasetError> {
        use tracel_client::station::dataset::QueryDatasetsRequest;

        let mut page = 0;
        loop {
            let response = self
                .client
                .datasets()
                .query(QueryDatasetsRequest {
                    page: Some(page),
                    per_page: Some(QUERY_PAGE_SIZE),
                    filter: None,
                })
                .map_err(|err| DatasetError::Client(Box::new(err)))?;

            if response.items.iter().any(|d| d.name == name) {
                return Ok(());
            }
            if response.items.len() < QUERY_PAGE_SIZE as usize {
                return Err(DatasetError::DatasetNotFound {
                    name: name.to_string(),
                });
            }
            page += 1;
        }
    }

    fn ensure_dataset_version_exists(&self, name: &str, version: u32) -> Result<(), DatasetError> {
        use tracel_client::station::dataset::QueryDatasetVersionsRequest;

        let mut page = 0;
        loop {
            let response = self
                .client
                .datasets()
                .versions(
                    name,
                    QueryDatasetVersionsRequest {
                        page: Some(page),
                        per_page: Some(QUERY_PAGE_SIZE),
                    },
                )
                .map_err(|err| DatasetError::Client(Box::new(err)))?;

            if response.items.iter().any(|v| v.version == version as i32) {
                return Ok(());
            }
            if response.items.len() < QUERY_PAGE_SIZE as usize {
                return Err(DatasetError::VersionNotFound {
                    name: name.to_string(),
                    version,
                });
            }
            page += 1;
        }
    }
}
