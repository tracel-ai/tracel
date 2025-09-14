use std::path::PathBuf;

use crate::artifacts::{ArtifactSources, IntoArtifactSources};
use crate::experiment::{ExperimentRun, ExperimentRunHandle};
use crate::record::ArtifactKind;
use burn::record::{
    FileRecorder, FullPrecisionSettings, NamedMpkBytesRecorder, Record, Recorder, RecorderError,
};
use burn::tensor::backend::Backend;
use serde::{Serialize, de::DeserializeOwned};

struct CheckpointRecordSources<B, R> {
    pub name: String,
    pub backend: std::marker::PhantomData<B>,
    pub record: R,
}

impl<B, R> CheckpointRecordSources<B, R> {
    pub fn new(name: String, record: R) -> Self {
        Self {
            name,
            backend: std::marker::PhantomData,
            record,
        }
    }
}

impl<B, R> IntoArtifactSources for CheckpointRecordSources<B, R>
where
    R: Record<B>,
    B: Backend,
{
    fn into_artifact_sources(self) -> ArtifactSources {
        let recorder = NamedMpkBytesRecorder::<FullPrecisionSettings>::default();
        let bytes = recorder
            .record(self.record, ())
            .expect("Failed to record item");
        ArtifactSources::new().add_bytes(bytes, self.name)
    }
}

/// A recorder that saves and loads data from a remote server using the [BurnCentralClientState](BurnCentralClientState).
#[derive(Debug, Clone)]
pub struct RemoteCheckpointRecorder {
    experiment_handle: ExperimentRunHandle,
}

impl RemoteCheckpointRecorder {
    pub fn new(experiment: &ExperimentRun) -> Self {
        Self {
            experiment_handle: experiment.handle(),
        }
    }
}

impl<B: Backend> FileRecorder<B> for RemoteCheckpointRecorder {
    fn file_extension() -> &'static str {
        "mpk"
    }
}

impl Default for RemoteCheckpointRecorder {
    fn default() -> Self {
        unimplemented!("Default is not implemented for RemoteRecorder, as it requires a client.")
    }
}

impl<B: Backend> Recorder<B> for RemoteCheckpointRecorder {
    type Settings = FullPrecisionSettings;
    type RecordArgs = PathBuf;
    type RecordOutput = ();
    type LoadArgs = PathBuf;

    fn record<R>(
        &self,
        record: R,
        args: Self::RecordArgs,
    ) -> Result<Self::RecordOutput, RecorderError>
    where
        R: Record<B>,
    {
        let file_name = args
            .file_name()
            .ok_or(RecorderError::Unknown(
                "File name should be present".to_string(),
            ))?
            .to_str()
            .ok_or(RecorderError::Unknown(
                "File name should be a valid string".to_string(),
            ))?;
        self.experiment_handle
            .try_log_artifact2(
                file_name,
                ArtifactKind::Other,
                CheckpointRecordSources::new(file_name.to_owned(), record),
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to record artifact: {e}")))
    }

    fn load<R>(&self, args: Self::LoadArgs, device: &B::Device) -> Result<R, RecorderError>
    where
        R: Record<B>,
    {
        self.experiment_handle
            .load_artifact::<B, R>(
                args.file_name()
                    .ok_or(RecorderError::Unknown(
                        "File name should be present".to_string(),
                    ))?
                    .to_str()
                    .ok_or(RecorderError::Unknown(
                        "File name should be a valid string".to_string(),
                    ))?,
                device,
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to load artifact: {e}")))
    }

    fn save_item<I: Serialize>(
        &self,
        _item: I,
        _args: Self::RecordArgs,
    ) -> Result<Self::RecordOutput, RecorderError> {
        Err(RecorderError::Unknown(
            "Saving items directly is not supported by RemoteCheckpointRecorder".to_string(),
        ))
    }

    fn load_item<I>(&self, _args: &mut Self::LoadArgs) -> Result<I, RecorderError>
    where
        I: DeserializeOwned,
    {
        Err(RecorderError::Unknown(
            "Loading items directly is not supported by RemoteCheckpointRecorder".to_string(),
        ))
    }
}
