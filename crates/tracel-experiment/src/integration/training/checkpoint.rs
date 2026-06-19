use burn::train::checkpoint::Checkpoint;
use burn::train::checkpoint::Checkpointer;
use burn::train::checkpoint::CheckpointerError;
use std::fmt;

use crate::ArtifactKind;
use crate::ExperimentId;
use crate::ExperimentRunHandle;
use burn::tensor::Bytes;
use serde::Deserialize;
use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

struct CheckpointRecordSources<C: Checkpoint> {
    pub checkpoint: C,
}

impl<C: Checkpoint> CheckpointRecordSources<C> {
    pub fn new(checkpoint: C) -> Self {
        Self { checkpoint }
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
            .checkpoint
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
        Ok(Self::new(Box::new(record)))
    }
}

/// Experiment-backed implementation of Burn's [`Checkpointer`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::checkpointers`] when you
/// already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
#[derive(Clone)]
pub struct ExperimentCheckpointer {
    experiment_handle: ExperimentRunHandle,
    file_name: String,
    restore_from: Option<ExperimentId>,
}

impl fmt::Debug for ExperimentCheckpointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExperimentCheckpointer")
            .finish_non_exhaustive()
    }
}

impl ExperimentCheckpointer {
    /// Create a checkpointer backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>, file_name: String) -> Self {
        Self {
            experiment_handle: experiment.into(),
            file_name,
            restore_from: None,
        }
    }

    /// Set a source experiment to restore checkpoints from.
    ///
    /// When set, `restore` loads artifacts from the given experiment instead of
    /// the current one. This is needed when resuming training across runs, since
    /// each run creates a new experiment that starts with no artifacts.
    pub fn with_restore_from(mut self, experiment_id: impl Into<ExperimentId>) -> Self {
        self.restore_from = Some(experiment_id.into());
        self
    }

    fn full_path_name(&self, epoch: usize) -> String {
        format!("{}-{}.bpk", self.file_name, epoch)
    }
}

impl Default for ExperimentCheckpointer {
    fn default() -> Self {
        unimplemented!(
            "Default is not implemented for ExperimentCheckpointer, as it requires an experiment run."
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
            name: self.full_path_name(epoch),
        };
        self.experiment_handle
            .save_artifact(
                self.full_path_name(epoch),
                ArtifactKind::Other,
                CheckpointRecordSources::new(Box::new(record)),
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
            name: self.full_path_name(epoch),
        };
        let source_id = self
            .restore_from
            .clone()
            .unwrap_or_else(|| self.experiment_handle.id().clone());
        let artifact = self
            .experiment_handle
            .use_artifact::<CheckpointRecordSources<C>>(
                source_id,
                self.full_path_name(epoch),
                &settings,
            )
            .map_err(|e| CheckpointerError::Unknown(format!("Failed to load artifact: {e}")))?;
        Ok(artifact.checkpoint)
    }
}
