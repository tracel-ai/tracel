use std::{marker::PhantomData, path::PathBuf};

use burn::{
    record::{PrecisionSettings, RecorderError},
    tensor::backend::Backend,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::client::BurnCentralClientState;

/// The strategy to use when saving data.
#[derive(Debug, Clone)]
pub enum RecorderStrategy {
    Checkpoint,
    Final,
}

/// A recorder that saves and loads data from a remote server using the [BurnCentralClientState](BurnCentralClientState).
#[derive(Debug, Clone)]
pub struct RemoteRecorder<S: PrecisionSettings> {
    client: BurnCentralClientState,
    checkpointer: RecorderStrategy,
    _settings: PhantomData<S>,
}

impl<S: PrecisionSettings> RemoteRecorder<S> {
    fn new(client: BurnCentralClientState, checkpointer: RecorderStrategy) -> Self {
        Self {
            client,
            checkpointer,
            _settings: PhantomData,
        }
    }

    /// Create a new RemoteRecorder with the given [BurnCentralClientState].
    pub fn checkpoint(client: BurnCentralClientState) -> Self {
        Self::new(client, RecorderStrategy::Checkpoint)
    }

    /// Create a new RemoteRecorder with the given [BurnCentralClientState].
    /// This recorder will save the data as a final trained model.
    pub fn final_model(client: BurnCentralClientState) -> Self {
        Self::new(client, RecorderStrategy::Final)
    }
}

impl<B: Backend, S: PrecisionSettings> burn::record::FileRecorder<B> for RemoteRecorder<S> {
    fn file_extension() -> &'static str {
        "mpk"
    }
}

impl<S: PrecisionSettings> Default for RemoteRecorder<S> {
    fn default() -> Self {
        unimplemented!("Default is not implemented for RemoteRecorder, as it requires a client.")
    }
}

impl<B: Backend, S: PrecisionSettings> burn::record::Recorder<B> for RemoteRecorder<S> {
    type Settings = S;
    type RecordArgs = PathBuf;
    type RecordOutput = ();
    type LoadArgs = PathBuf;

    fn save_item<I: Serialize>(
        &self,
        item: I,
        mut file: Self::RecordArgs,
    ) -> Result<Self::RecordOutput, RecorderError> {
        let serialized_bytes =
            rmp_serde::encode::to_vec_named(&item).expect("Should be able to serialize.");

        match self.checkpointer {
            RecorderStrategy::Checkpoint => {
                file.set_extension(<Self as burn::record::FileRecorder<B>>::file_extension());
                let file_name = file
                    .file_name()
                    .ok_or(RecorderError::Unknown(
                        "File name should be present".to_string(),
                    ))?
                    .to_str()
                    .ok_or(RecorderError::Unknown(
                        "File name should be a valid string".to_string(),
                    ))?;
                self.client
                    .save_checkpoint_data(file_name, serialized_bytes.clone())
                    .map_err(|err| RecorderError::Unknown(err.to_string()))?;
            }
            RecorderStrategy::Final => {
                self.client
                    .save_final_model(serialized_bytes.clone())
                    .map_err(|err| RecorderError::Unknown(err.to_string()))?;
            }
        }

        Ok(())
    }

    fn load_item<I: DeserializeOwned>(
        &self,
        file: &mut Self::LoadArgs,
    ) -> Result<I, RecorderError> {
        let data = match self.checkpointer {
            RecorderStrategy::Checkpoint => {
                file.set_extension(<Self as burn::record::FileRecorder<B>>::file_extension());
                let file_name = file
                    .file_name()
                    .ok_or(RecorderError::Unknown(
                        "File name should be present".to_string(),
                    ))?
                    .to_str()
                    .ok_or(RecorderError::Unknown(
                        "File name should be a valid string".to_string(),
                    ))?;
                self.client
                    .load_checkpoint_data(file_name)
                    .map_err(|err| RecorderError::Unknown(err.to_string()))?
            }
            RecorderStrategy::Final => {
                unimplemented!("Final model loading is not implemented yet.")
            }
        };

        let item = rmp_serde::decode::from_slice(&data).expect("Should be able to deserialize.");
        Ok(item)
    }
}
