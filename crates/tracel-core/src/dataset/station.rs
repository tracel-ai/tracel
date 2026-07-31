use tracel_client::station::dataset::StreamDatasetVersionItemsRequest;
use tracel_client::{ApiErrorCode, ClientError};

use crate::backend::station::StationBackend;
use crate::dataset::{DatasetError, DatasetItemsPage, DatasetProvider};

impl DatasetProvider for StationBackend {
    fn get_items(
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
            .map_err(|err| describe_error(err, name, Some(version)))?;

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
            .map_err(|err| describe_error(err, name, Some(version)))
    }

    fn resolve_version(&self, name: &str) -> Result<u32, DatasetError> {
        let response = self
            .client
            .datasets()
            .get_latest_version(name)
            .map_err(|err| describe_error(err, name, None))?;

        u32::try_from(response.version).map_err(|_| {
            DatasetError::Client(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "dataset '{name}' latest version {} cannot be represented as a u32",
                    response.version
                ),
            )))
        })
    }
}

fn describe_error(err: ClientError, name: &str, version: Option<u32>) -> DatasetError {
    match err {
        ClientError::NotFoundWithCode(ApiErrorCode::Dataset) => DatasetError::DatasetNotFound {
            name: name.to_string(),
        },
        ClientError::NotFoundWithCode(ApiErrorCode::DatasetVersion) => match version {
            Some(version) => DatasetError::VersionNotFound {
                name: name.to_string(),
                version,
            },
            None => DatasetError::NoVersionsFound {
                name: name.to_string(),
            },
        },
        err => DatasetError::Client(Box::new(err)),
    }
}
