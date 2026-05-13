use crate::dataset::{AnnotationDataset, DatasetRef};
use burn_central_client::ClientError;
use burn_central_experiment::{ExperimentRun, error::ExperimentError};

/// High-level client for interacting with Burn Station.
pub struct Station {
    client: burn_central_client::StationClient,
}

#[derive(Debug, thiserror::Error)]
#[error("Failed to connect to Burn Station: {source}")]
pub struct StationConnectionError {
    #[source]
    source: ClientError,
}

impl Station {
    /// Create a new BurnStation client from a base URL.
    pub fn connect(base_url: &str) -> Result<Self, StationConnectionError> {
        let client = burn_central_client::StationClient::from_url(base_url.parse().unwrap());
        client
            .system()
            .health()
            .map_err(|e| StationConnectionError { source: e })?;
        Ok(Self { client })
    }

    pub fn create_experiment(&self, name: String) -> Result<ExperimentRun, ExperimentError> {
        ExperimentRun::station(self.client.clone(), name)
    }

    pub fn get_annotation_dataset(&self, dataset_ref: DatasetRef) -> AnnotationDataset {
        AnnotationDataset::new(self.client.clone(), dataset_ref)
    }
}
