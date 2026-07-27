use tracel_client::station::dataset::StreamDatasetVersionItemsRequest;
use tracel_client::{ApiErrorCode, ClientError};

use crate::backend::station::StationBackend;
use crate::dataset::{DatasetError, DatasetItemsPage, DatasetProvider};

impl DatasetProvider for StationBackend {
    fn stream_items(
        &self,
        name: &str,
        version: u32,
        index: Option<u64>,
        limit: Option<u32>,
    ) -> Result<DatasetItemsPage, DatasetError> {
        let response = self
            .client
            .datasets()
            .stream_items(
                name,
                version,
                StreamDatasetVersionItemsRequest { index, limit },
            )
            .map_err(|err| Self::describe_error(err, name, version))?;

        Ok(DatasetItemsPage {
            items: response
                .items
                .into_iter()
                .map(|item| item.payload)
                .collect(),
        })
    }

    fn item_count(&self, name: &str, version: u32) -> Result<u64, DatasetError> {
        self.client
            .datasets()
            .get_version(name, version)
            .map(|response| response.item_count)
            .map_err(|err| Self::describe_error(err, name, version))
    }
}

impl StationBackend {
    fn describe_error(err: ClientError, name: &str, version: u32) -> DatasetError {
        match err {
            ClientError::NotFoundWithCode(ApiErrorCode::Dataset) => DatasetError::DatasetNotFound {
                name: name.to_string(),
            },
            ClientError::NotFoundWithCode(ApiErrorCode::DatasetVersion) => {
                DatasetError::VersionNotFound {
                    name: name.to_string(),
                    version,
                }
            }
            err => DatasetError::Client(Box::new(err)),
        }
    }
}
