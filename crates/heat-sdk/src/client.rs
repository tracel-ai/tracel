use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;

use crate::error::HeatSDKError;

pub enum AccessMode {
    Read,
    Write,
}

/// Configuration for the HeatClient. Can be created using [HeatClientConfigBuilder], which is created using the [HeatClientConfig::builder] method.
#[derive(Debug)]
pub struct HeatClientConfig {
    /// The endpoint of the Heat API
    pub endpoint: String,
    /// The API key to authenticate with the Heat API.
    pub api_key: String, // not used at the moment
    /// The number of retries to attempt when connecting to the Heat API.
    pub num_retries: u8,
}

impl HeatClientConfig {
    /// Create a new [HeatClientConfigBuilder] with the given API key.
    pub fn builder(api_key: impl Into<String>) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder::new(api_key)
    }
}

/// Builder for the HeatClientConfig
pub struct HeatClientConfigBuilder {
    config: HeatClientConfig,
}

impl HeatClientConfigBuilder {
    pub(crate) fn new(api_key: impl Into<String>) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder {
            config: HeatClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                api_key: api_key.into(),
                num_retries: 3,
            },
        }
    }

    /// Set the endpoint of the Heat API
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> HeatClientConfigBuilder {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set the number of retries to attempt when connecting to the Heat API
    pub fn with_num_retries(mut self, num_retries: u8) -> HeatClientConfigBuilder {
        self.config.num_retries = num_retries;
        self
    }

    /// Build the HeatClientConfig
    pub fn build(self) -> HeatClientConfig {
        self.config
    }
}

/// The HeatClient is used to interact with the Heat API. It is required for all interactions with the Heat API.
#[derive(Debug)]
pub struct HeatClient {
    config: HeatClientConfig,

    http_client: reqwest::blocking::Client,
}

/// The HeatClientState is a shared state for the HeatClient. It is used to ensure that the client is thread-safe when used in Burn.
type HeatClientState = Arc<HeatClient>;

impl HeatClient {
    fn new(config: HeatClientConfig) -> HeatClient {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Client should be created.");

        HeatClient {
            config,
            http_client,
        }
    }

    fn health_check(&self) -> Result<(), reqwest::Error> {
        let url = format!("{}/health", self.get_endpoint());
        self.http_client.get(url).send()?;

        Ok(())
    }

    /// Create a new HeatClient with the given configuration.
    pub fn create(config: HeatClientConfig) -> Result<HeatClientState, HeatSDKError> {
        let client_state = Arc::new(HeatClient::new(config));

        // Try to connect to the api, if it fails, return an error
        for i in 0..=client_state.config.num_retries {
            let res = client_state.health_check();

            match res {
                Ok(_) => break,
                Err(e) => {
                    if i == client_state.config.num_retries {
                        return Err(HeatSDKError::ServerTimeoutError(e.to_string()));
                    }
                }
            }
        }

        Ok(client_state)
    }

    /// Get the endpoint of the Heat API
    pub fn get_endpoint(&self) -> String {
        self.config.endpoint.clone()
    }

    /// Get the API key of the Heat API
    pub fn get_api_key(&self) -> String {
        self.config.api_key.clone()
    }

    /// Get the number of retries to attempt when connecting to the Heat API
    pub fn get_num_retries(&self) -> u8 {
        self.config.num_retries
    }

    fn request_checkpoint_url(
        &self,
        path: &str,
        access: AccessMode,
    ) -> Result<String, reqwest::Error> {
        let url = format!("{}/checkpoints", self.get_endpoint());

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());

        #[derive(Deserialize)]
        struct CheckpointURLResponse {
            url: String,
        }

        let response = match access {
            AccessMode::Read => self.http_client.get(url),
            AccessMode::Write => self.http_client.post(url),
        }
        .json(&body)
        .send()?
        .json::<CheckpointURLResponse>()?;

        Ok(response.url)
    }

    fn upload_checkpoint(&self, url: &str, checkpoint: Vec<u8>) -> Result<(), reqwest::Error> {
        self.http_client.put(url).body(checkpoint).send()?;

        Ok(())
    }

    fn download_checkpoint(&self, url: &str) -> Result<Vec<u8>, reqwest::Error> {
        let response = self.http_client.get(url).send()?.bytes()?;

        Ok(response.to_vec())
    }

    /// Save checkpoint data to the Heat API.
    pub fn save_checkpoint_data(
        &self,
        path: &str,
        checkpoint: Vec<u8>,
    ) -> Result<(), HeatSDKError> {
        let url = self.request_checkpoint_url(path, AccessMode::Write)?;
        self.upload_checkpoint(&url, checkpoint)?;

        Ok(())
    }

    /// Load checkpoint data from the Heat API
    pub fn load_checkpoint_data(&self, path: &str) -> Result<Vec<u8>, HeatSDKError> {
        let url = self.request_checkpoint_url(path, AccessMode::Read)?;
        let response = self.download_checkpoint(&url)?;

        Ok(response.to_vec())
    }
}
