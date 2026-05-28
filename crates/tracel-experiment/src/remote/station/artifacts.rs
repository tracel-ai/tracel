use burn_central_client::StationClient;
use tracel_artifact::bundle::FsBundle;

use crate::{
    ArtifactKind, ExperimentId,
    reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact},
    remote::{
        base::{ArtifactUploadError, ArtifactUploader},
        station::{ExperimentArtifactClient, ExperimentPath, StationExperimentId},
    },
};

pub struct StationArtifactReader {
    client: StationClient,
}

impl StationArtifactReader {
    pub fn new(client: StationClient) -> Self {
        Self { client }
    }
}

impl ExperimentArtifactReader for StationArtifactReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let id = StationExperimentId::from_experiment_id(&experiment_id)
            .ok_or_else(|| ExperimentReaderError::new("Invalid experiment ID format"))?;

        let experiment_path = ExperimentPath::new(id.num());
        let scope = ExperimentArtifactClient::new(self.client.clone(), experiment_path);
        let artifact = scope.fetch(name).map_err(|err| {
            ExperimentReaderError::with_source("Failed to resolve experiment artifact", err)
        })?;

        scope
            .download(name)
            .map_err(|err| {
                ExperimentReaderError::with_source("Failed to download experiment artifact", err)
            })
            .map(|bundle| {
                LoadedArtifact::new(
                    ArtifactRef {
                        id: artifact.id.to_string(),
                        name: name.to_string(),
                    },
                    bundle,
                )
            })
    }
}

pub struct StationArtifactUploader {
    client: ExperimentArtifactClient,
}

impl StationArtifactUploader {
    pub fn new(client: StationClient, exp_path: ExperimentPath) -> Self {
        Self {
            client: ExperimentArtifactClient::new(client, exp_path),
        }
    }
}

impl ArtifactUploader for StationArtifactUploader {
    fn upload(
        &self,
        name: &str,
        kind: ArtifactKind,
        bundle: &FsBundle,
    ) -> Result<(), ArtifactUploadError> {
        self.client
            .upload(name, kind, bundle)
            .map(|_| ())
            .map_err(|e| ArtifactUploadError {
                message: format!("Failed to upload artifact '{}'", name),
                source: Some(Box::new(e)),
            })
    }
}
