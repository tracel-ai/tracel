use std::sync::Arc;

use tracel_artifact::bundle::FsBundle;

use tracel_experiment::{
    ArtifactKind, ExperimentId,
    reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact},
};

use tracel_experiment_remote::{ArtifactUploadError, ArtifactUploader};

use super::{ExperimentArtifactClient, ExperimentPath};
use crate::station::StationInner;

pub struct StationArtifactReader {
    station: Arc<StationInner>,
}

impl StationArtifactReader {
    pub fn new(station: Arc<StationInner>) -> Self {
        Self { station }
    }
}

impl ExperimentArtifactReader for StationArtifactReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let num = experiment_id
            .parse::<i32>()
            .ok_or_else(|| ExperimentReaderError::new("Invalid experiment ID format"))?;

        let experiment_path = ExperimentPath::new(num);
        let scope = ExperimentArtifactClient::new(Arc::clone(&self.station), experiment_path);
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
    pub fn new(station: Arc<StationInner>, exp_path: ExperimentPath) -> Self {
        Self {
            client: ExperimentArtifactClient::new(station, exp_path),
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
