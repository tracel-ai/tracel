use burn::train::checkpoint::Checkpoint;
use burn::train::checkpoint::Checkpointer;
use burn::train::checkpoint::CheckpointerError;
use std::fmt;

use crate::ArtifactKind;
use crate::ExperimentRunHandle;
use burn::tensor::Bytes;
use serde::Deserialize;
use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

struct CheckpointRecordSources<C: Checkpoint> {
    pub record: C,
}

impl<C: Checkpoint> CheckpointRecordSources<C> {
    pub fn new(record: C) -> Self {
        Self { record }
    }
}

#[derive(Serialize, Deserialize)]
struct CheckpointRecordArtifactSettings {
    pub name: String,
}

impl Default for CheckpointRecordArtifactSettings {
    fn default() -> Self {
        Self {
            name: "checkpoint.mpk".to_string(),
        }
    }
}

impl<C: Checkpoint> BundleEncode for CheckpointRecordSources<C> {
    type Settings = CheckpointRecordArtifactSettings;
    type Error = String;
    fn encode<E: BundleSink>(
        self,
        sink: &mut E,
        settings: &Self::Settings,
    ) -> Result<(), Self::Error> {
        let bytes = self
            .record
            .checkpoint_into_bytes()
            .map_err(|e| format!("Failed to record to bytes: {}", e))?;

        sink.put_bytes(&settings.name, &bytes)
            .map_err(|e| format!("Failed to write bytes to sink: {}", e))?;
        Ok(())
    }
}

impl<C: Checkpoint> BundleDecode for CheckpointRecordSources<C> {
    type Settings = CheckpointRecordArtifactSettings;
    type Error = String;

    fn decode<I: BundleSource>(source: &I, settings: &Self::Settings) -> Result<Self, Self::Error> {
        let mut reader = source.open(&settings.name).map_err(|e| {
            format!(
                "Failed to get reader for checkpoint artifact {}: {}",
                settings.name, e
            )
        })?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|e| {
            format!(
                "Failed to read bytes for checkpoint artifact {}: {}",
                settings.name, e
            )
        })?;
        let record = C::checkpoint_from_bytes(Bytes::from_bytes_vec(bytes))
            .map_err(|e| format!("Failed to load record from bytes: {}", e))?;
        Ok(Self::new(record))
    }
}

/// Experiment-backed implementation of Burn's [`Recorder`] and [`FileRecorder`] traits.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::checkpoint_recorder`] when you
/// already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
#[derive(Clone)]
pub struct ExperimentCheckpointer {
    experiment_handle: ExperimentRunHandle,
    file_name: String,
}

impl fmt::Debug for ExperimentCheckpointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExperimentCheckpointRecorder")
            .finish_non_exhaustive()
    }
}

impl ExperimentCheckpointer {
    /// Create a recorder backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>, file_name: String) -> Self {
        Self {
            experiment_handle: experiment.into(),
            file_name,
        }
    }

    fn path_for_epoch(&self, epoch: usize) -> String {
        format!("{}-{}.bpk", self.file_name, epoch)
    }
}

impl Default for ExperimentCheckpointer {
    fn default() -> Self {
        unimplemented!(
            "Default is not implemented for ExperimentCheckpointRecorder, as it requires an experiment run."
        )
    }
}

impl<C: Checkpoint> Checkpointer<C> for ExperimentCheckpointer {
    fn save(
        &self,
        epoch: usize,
        record: C,
    ) -> Result<(), burn::train::checkpoint::CheckpointerError> {
        let settings = CheckpointRecordArtifactSettings {
            name: self.path_for_epoch(epoch),
        };
        self.experiment_handle
            .save_artifact(
                self.file_name.clone(),
                ArtifactKind::Other,
                CheckpointRecordSources::new(record),
                &settings,
            )
            .map_err(|e| {
                burn::train::checkpoint::CheckpointerError::Unknown(format!(
                    "Failed to save artifact: {e}"
                ))
            })
    }

    fn delete(&self, _epoch: usize) -> Result<(), burn::train::checkpoint::CheckpointerError> {
        Ok(())
    }

    fn restore(&self, epoch: usize) -> Result<C, burn::train::checkpoint::CheckpointerError> {
        let settings = CheckpointRecordArtifactSettings {
            name: self.path_for_epoch(epoch),
        };
        let artifact = self
            .experiment_handle
            .use_artifact::<CheckpointRecordSources<C>>(
                self.experiment_handle.id().clone(),
                self.file_name.clone(),
                &settings,
            )
            .map_err(|e| CheckpointerError::Unknown(format!("Failed to load artifact: {e}")))?;
        Ok(artifact.record)
    }
}
