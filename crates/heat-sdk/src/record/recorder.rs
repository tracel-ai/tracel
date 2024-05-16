use std::{marker::PhantomData, path::PathBuf};

use burn::{
    record::{PrecisionSettings, RecorderError},
    tensor::backend::Backend,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::client::HeatClientState;

#[derive(Debug, Clone)]
pub struct RemoteRecorder<S: PrecisionSettings> {
    client: HeatClientState,
    _settings: PhantomData<S>,
}

impl<S: PrecisionSettings> RemoteRecorder<S> {
    pub fn new(client: HeatClientState) -> Self {
        Self {
            client,
            _settings: PhantomData,
        }
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

        self.client
            .as_ref()
            .lock()
            .map_err(|err| RecorderError::Unknown(err.to_string()))?
            .save_checkpoint_data(&path, serialized_bytes.clone())
            .map_err(|err| RecorderError::Unknown(err.to_string()))?;

        Ok(())
    }

    fn load_item<I: DeserializeOwned>(&self, mut file: Self::LoadArgs) -> Result<I, RecorderError> {
        file.set_extension(<Self as burn::record::FileRecorder<B>>::file_extension());
        let path = file
            .to_str()
            .expect("file should be a valid string.")
            .to_string();
        let data = self
            .client
            .as_ref()
            .lock()
            .map_err(|err| RecorderError::Unknown(err.to_string()))?
            .load_checkpoint_data(&path)
            .map_err(|err| RecorderError::Unknown(err.to_string()))?;

        let item = rmp_serde::decode::from_slice(&data).expect("Should be able to deserialize.");
        Ok(item)
    }
}
