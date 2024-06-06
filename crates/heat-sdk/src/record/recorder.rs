use std::{marker::PhantomData, path::PathBuf};

use burn::{
    record::{PrecisionSettings, RecorderError},
    tensor::backend::Backend,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::client::HeatClientState;

/// The strategy to use when saving data.
#[derive(Debug, Clone)]
pub enum RecorderStrategy {
    Checkpoint,
    Final,
}

/// A recorder that saves and loads data from a remote server using the [HeatClientState](HeatClientState).
#[derive(Debug, Clone)]
pub struct RemoteRecorder<S: PrecisionSettings> {
    client: HeatClientState,
    checkpointer: RecorderStrategy,
    _settings: PhantomData<S>,
}

impl<S: PrecisionSettings> RemoteRecorder<S> {
    fn new(client: HeatClientState, checkpointer: RecorderStrategy) -> Self {
        Self {
            client,
            checkpointer,
            _settings: PhantomData,
        }
    }

    /// Create a new RemoteRecorder with the given [HeatClientState].
    pub fn checkpoint(client: HeatClientState) -> Self {
        Self::new(client, RecorderStrategy::Checkpoint)
    }

    /// Create a new RemoteRecorder with the given [HeatClientState].
    /// This recorder will save the data as a final trained model.
    pub fn final_model(client: HeatClientState) -> Self {
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
        file.set_extension(<Self as burn::record::FileRecorder<B>>::file_extension());
        let path = file
            .to_str()
            .expect("file should be a valid string.")
            .to_string();
        let serialized_bytes =
            rmp_serde::encode::to_vec_named(&item).expect("Should be able to serialize.");

        match self.checkpointer {
            RecorderStrategy::Checkpoint => {
                self.client
                    .save_checkpoint_data(&path, serialized_bytes.clone())
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

    fn load_item<I: DeserializeOwned>(&self, mut file: Self::LoadArgs) -> Result<I, RecorderError> {
        file.set_extension(<Self as burn::record::FileRecorder<B>>::file_extension());
        let path = file
            .to_str()
            .expect("file should be a valid string.")
            .to_string();

        let data = match self.checkpointer {
            RecorderStrategy::Checkpoint => self
                .client
                .load_checkpoint_data(&path)
                .map_err(|err| RecorderError::Unknown(err.to_string()))?,
            RecorderStrategy::Final => {
                unimplemented!("Final model loading is not implemented yet.")
            }
        };

        let item = rmp_serde::decode::from_slice(&data).expect("Should be able to deserialize.");
        Ok(item)
    }
}
