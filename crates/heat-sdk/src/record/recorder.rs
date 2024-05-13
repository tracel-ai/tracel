use std::{io::{BufReader, BufWriter}, marker::PhantomData};

use burn::record::RecorderError;

use burn::{record::PrecisionSettings, tensor::backend::Backend};
use derive_new::new;
use std::fs::File;
use std::path::PathBuf;

use rmp_serde;
use serde_json;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use reqwest; // uwu

use std::collections::HashMap;


#[derive(new, Debug, Default, Clone)]
pub struct RemoteRecorder<S: PrecisionSettings> {
    _settings: PhantomData<S>,
}

macro_rules! str2reader {
    (
        $file:expr
    ) => {{
        $file.set_extension("mpk");
        let path = $file.as_path();

        File::open(path)
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => RecorderError::FileNotFound(err.to_string()),
                _ => RecorderError::Unknown(err.to_string()),
            })
            .map(|file| BufReader::new(file))
    }};
}

macro_rules! str2writer {
    (
        $file:expr
    ) => {{
        $file.set_extension("mpk");
        let path = $file.as_path();
        log::info!("Bruh Writing to {:?}", path);

        // Add parent directories if they don't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        if path.exists() {
            log::info!("File exists, replacing");
            std::fs::remove_file(path).map_err(|err| RecorderError::Unknown(err.to_string()))?;
        }

        File::create(path)
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => RecorderError::FileNotFound(err.to_string()),
                _ => RecorderError::Unknown(err.to_string()),
            })
            .map(|file| BufWriter::new(file))
    }};
}

impl<B: Backend, S: PrecisionSettings> burn::record::FileRecorder<B> for RemoteRecorder<S> {
    fn file_extension() -> &'static str {
        "mpk"
    }
}

#[derive(Deserialize)]
struct URL {
    url: String,
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
    ) -> Result<Self::RecordOutput, RecorderError>
    {
        let serialized_bytes = rmp_serde::encode::to_vec_named(&item).expect("Should be able to serialize.");

        let mut body = HashMap::new();
        body.insert("file_path", file.to_str().unwrap().to_string());

        let client = reqwest::blocking::Client::new();
        let res = client.post("http://localhost:8080/checkpoints")
            .json(&body)
            .send()
            .expect("Failed to send request");

        let url: URL = res.json().expect("Failed to parse JSON");
        let mut path = PathBuf::from(url.url);

        let mut writer = str2writer!(path)?;
            
        rmp_serde::encode::write_named(&mut writer, &item)
            .map_err(|err| RecorderError::Unknown(err.to_string()))?;
    
        Ok(())
    }
    
    fn load_item<I: DeserializeOwned>(&self, mut file: Self::LoadArgs) -> Result<I, RecorderError> 
    {
        let reader = str2reader!(file)?;
        let state = rmp_serde::decode::from_read(reader)
            .map_err(|err| RecorderError::Unknown(err.to_string()))?;
    
        Ok(state)
    }
}
