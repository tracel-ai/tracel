use std::marker::PhantomData;

use burn::record::RecorderError;

use burn::{record::PrecisionSettings, tensor::backend::Backend};
use derive_new::new;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use rmp_serde;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use reqwest; // uwu

use std::collections::HashMap;

use std::sync::Arc;
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
        file: Self::RecordArgs,
    ) -> Result<Self::RecordOutput, RecorderError> {

        let path = file.to_str().unwrap().to_string();
        let serialized_bytes =
            rmp_serde::encode::to_vec_named(&item).expect("Should be able to serialize.");

        self.client.as_ref()
            .expect("Client not initialized")
            .save_checkpoint_data(&path, serialized_bytes)
            .map_err(|err| RecorderError::Unknown(err.to_string()))
    }

    fn load_item<I: DeserializeOwned>(&self, mut file: Self::LoadArgs) -> Result<I, RecorderError> {
        unimplemented!("RemoteRecorder does not yet support loading")
    }
}
