use std::sync::Arc;

use tracel_artifact::bundle::FsBundle;

use tracel_experiment::{
    ArtifactKind, ExperimentId,
    reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact},
};

use tracel_experiment_remote::{ArtifactUploadError, ArtifactUploader};
use tracel_task::Job;

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
        name: String,
    ) -> Job<LoadedArtifact, ExperimentReaderError> {
        let Some(num) = experiment_id.parse::<i32>() else {
            return Job::failed(ExperimentReaderError::new("Invalid experiment ID format"));
        };

        let scope =
            ExperimentArtifactClient::new(Arc::clone(&self.station), ExperimentPath::new(num));
        self.station.attach(async move {
            let (artifact, bundle) = scope.download(&name).await.map_err(|err| {
                ExperimentReaderError::with_source("Failed to download experiment artifact", err)
            })?;

            Ok(LoadedArtifact::new(
                ArtifactRef {
                    id: artifact.id.to_string(),
                    name,
                },
                bundle,
            ))
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
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Job<(), ArtifactUploadError> {
        let client = self.client.clone();
        self.client.station.attach(async move {
            client
                .upload(name.clone(), kind, bundle)
                .await
                .map(drop)
                .map_err(|e| ArtifactUploadError {
                    message: format!("Failed to upload artifact '{name}'"),
                    source: Some(Box::new(e)),
                })
        })
    }
}
