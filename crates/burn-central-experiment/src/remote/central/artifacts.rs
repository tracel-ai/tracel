use burn_central_artifact::bundle::FsBundle;
use burn_central_client::Client;

use crate::{
    ArtifactKind, ExperimentId,
    reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact},
    remote::{
        base::{ArtifactUploadError, ArtifactUploader},
        central::{CentralExperimentId, ExperimentArtifactClient, ExperimentPath},
    },
};

pub struct CentralArtifactReader {
    client: Client,
    exp_path: ExperimentPath,
}

impl CentralArtifactReader {
    pub fn new(client: Client, exp_path: ExperimentPath) -> Self {
        Self { client, exp_path }
    }
}

impl ExperimentArtifactReader for CentralArtifactReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let id = CentralExperimentId::from_experiment_id(&experiment_id)
            .ok_or_else(|| ExperimentReaderError::new("Invalid experiment ID format"))?;

        let experiment_path = ExperimentPath::new(
            self.exp_path.owner_name().to_string(),
            self.exp_path.project_name().to_string(),
            id.num(),
        );
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

pub struct CentralArtifactUploader {
    client: ExperimentArtifactClient,
}

impl CentralArtifactUploader {
    pub fn new(client: Client, exp_path: ExperimentPath) -> Self {
        Self {
            client: ExperimentArtifactClient::new(client, exp_path),
        }
    }
}

impl ArtifactUploader for CentralArtifactUploader {
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
