use std::{marker::PhantomData, path::PathBuf, sync::Arc};

use burn::{
    record::{PrecisionSettings, RecorderError},
    tensor::backend::Backend,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::client;

#[derive(Debug, Default, Clone)]
pub struct RemoteRecorder<S: PrecisionSettings> {
    client: Option<Arc<client::HeatClient>>,
    _settings: PhantomData<S>,
}

impl<S: PrecisionSettings> RemoteRecorder<S> {
    pub fn new(client: Arc<client::HeatClient>) -> Self {
        Self {
            client: Some(client),
            _settings: PhantomData,
        }
    }
}

impl<B: Backend, S: PrecisionSettings> burn::record::FileRecorder<B> for RemoteRecorder<S> {
    fn file_extension() -> &'static str {
        "mpk"
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
            .expect("Client must be initialized.")
            .save_checkpoint_data(&path, serialized_bytes.clone())
            .map_err(|err| RecorderError::Unknown(err.to_string()))?;

        Ok(())
    }

    fn load_item<I: DeserializeOwned>(&self, mut file: Self::LoadArgs) -> Result<I, RecorderError> {
        unimplemented!("RemoteRecorder does not yet support loading")
    }
}
