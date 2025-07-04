use std::{marker::PhantomData, path::PathBuf};

use crate::ArtifactKind;
use crate::experiment::{Experiment, ExperimentHandle};
use burn::record::Record;
use burn::{
    record::{PrecisionSettings, RecorderError},
    tensor::backend::Backend,
};
use serde::{Serialize, de::DeserializeOwned};

/// A recorder that saves and loads data from a remote server using the [BurnCentralClientState](BurnCentralClientState).
#[derive(Debug, Clone)]
pub struct RemoteCheckpointRecorder<S: PrecisionSettings> {
    experiment_handle: ExperimentHandle,
    _settings: PhantomData<S>,
}

impl<S: PrecisionSettings> RemoteCheckpointRecorder<S> {
    fn new(experiment: &Experiment) -> Self {
        Self {
            experiment_handle: experiment.handle(),
            _settings: PhantomData,
        }
    }
}

impl<B: Backend, S: PrecisionSettings> burn::record::FileRecorder<B>
    for RemoteCheckpointRecorder<S>
{
    fn file_extension() -> &'static str {
        "mpk"
    }
}

impl<S: PrecisionSettings> Default for RemoteCheckpointRecorder<S> {
    fn default() -> Self {
        unimplemented!("Default is not implemented for RemoteRecorder, as it requires a client.")
    }
}

impl<B: Backend, S: PrecisionSettings> burn::record::Recorder<B> for RemoteCheckpointRecorder<S> {
    type Settings = S;
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
        self.experiment_handle.log_artifact(
            args.file_name()
                .ok_or(RecorderError::Unknown(
                    "File name should be present".to_string(),
                ))?
                .to_str()
                .ok_or(RecorderError::Unknown(
                    "File name should be a valid string".to_string(),
                ))?,
            ArtifactKind::Checkpoint,
            record,
        );
        Ok(())
    }

    fn load<R>(&self, args: Self::LoadArgs, device: &B::Device) -> Result<R, RecorderError>
    where
        R: Record<B>,
    {
        let record = self.experiment_handle.load_artifact::<B, R>(
            args.file_name()
                .ok_or(RecorderError::Unknown(
                    "File name should be present".to_string(),
                ))?
                .to_str()
                .ok_or(RecorderError::Unknown(
                    "File name should be a valid string".to_string(),
                ))?,
            device.clone(),
        );
        record
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
