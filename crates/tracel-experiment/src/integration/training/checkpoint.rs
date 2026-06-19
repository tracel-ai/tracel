use burn::train::checkpoint::Checkpoint;
use burn::train::checkpoint::Checkpointer;
use burn::train::checkpoint::CheckpointerError;
use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use thiserror;

use crate::ArtifactKind;
use crate::ExperimentRunHandle;
use burn::store::ModuleRecord;
use burn::tensor::Bytes;
use serde::Deserialize;
use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

#[derive(thiserror::Error, Debug)]
pub enum ExperimentCheckpointError {
    #[error("File name should be a valid string")]
    InvalidFileName,

    #[error("File name should be present")]
    MissingFileName,

    #[error("{0} items directly is not supported by ExperimentCheckpointRecorder")]
    NotSupported(String),
}

struct CheckpointRecordSources {
    pub record: Box<dyn Any>,
}

impl CheckpointRecordSources {
    pub fn new(record: Box<dyn Any>) -> Self {
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

impl BundleEncode for CheckpointRecordSources {
    type Settings = CheckpointRecordArtifactSettings;
    type Error = String;
    fn encode<E: BundleSink>(
        self,
        sink: &mut E,
        settings: &Self::Settings,
    ) -> Result<(), Self::Error> {
        let bytes = self
            .record
            .downcast::<ModuleRecord>()
            .expect("Should be a ModuleRecord")
            .into_bytes()
            .map_err(|e| format!("Failed to record to bytes: {}", e))?;

        sink.put_bytes(&settings.name, &bytes)
            .map_err(|e| format!("Failed to write bytes to sink: {}", e))?;
        Ok(())
    }
}

impl BundleDecode for CheckpointRecordSources {
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
        let record = ModuleRecord::from_bytes(Bytes::from_bytes_vec(bytes))
            .map_err(|e| format!("Failed to load record from bytes: {}", e))?;
        Ok(Self::new(Box::new(record)))
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
    pub fn try_new(
        experiment: impl Into<ExperimentRunHandle>,
        path: PathBuf,
    ) -> Result<Self, ExperimentCheckpointError> {
        let file_name = path
            .file_name()
            .ok_or(ExperimentCheckpointError::MissingFileName)?
            .to_str()
            .ok_or(ExperimentCheckpointError::InvalidFileName)?
            .to_string();
        Ok(Self {
            experiment_handle: experiment.into(),
            file_name,
        })
    }
}

// impl FileRecorder for ExperimentCheckpointRecorder {
//     fn file_extension() -> &'static str {
//         "mpk"
//     }
// }

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
        _epoch: usize,
        record: C,
    ) -> Result<(), burn::train::checkpoint::CheckpointerError> {
        let settings = CheckpointRecordArtifactSettings {
            name: self.file_name.clone(),
        };
        self.experiment_handle
            .save_artifact(
                self.file_name.clone(),
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

    fn restore(&self, _epoch: usize) -> Result<C, burn::train::checkpoint::CheckpointerError> {
        let settings = CheckpointRecordArtifactSettings {
            name: self.file_name.clone(),
        };
        let artifact = self
            .experiment_handle
            .use_artifact::<CheckpointRecordSources>(
                self.experiment_handle.id().clone(),
                self.file_name.clone(),
                &settings,
            )
            .map_err(|e| CheckpointerError::Unknown(format!("Failed to load artifact: {e}")))?;
        Ok(*artifact.record.downcast::<C>().expect(""))
    }
}
