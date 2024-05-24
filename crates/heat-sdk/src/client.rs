use std::sync::Mutex;
use std::{collections::HashMap, sync::Arc};

use serde::Deserialize;

use crate::error::HeatSdkError;
use crate::experiment::Experiment;
use crate::websocket::WebSocketClient;

enum AccessMode {
    Read,
    Write,
}

#[derive(Deserialize)]
struct URLResponse {
    url: String,
}

// enum Credentials {
//     ApiKey(String),
//     Login {
//         username: String,
//         password: String,
//     },
// }

/// Configuration for the HeatClient. Can be created using [HeatClientConfigBuilder], which is created using the [HeatClientConfig::builder] method.
#[derive(Debug, Clone)]
pub struct HeatClientConfig {
    /// The endpoint of the Heat API
    pub endpoint: String,
    /// The endpoint of the Heat API
    pub api_key: String, // not used yet, but will be used for authentication through the Heat backend API
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
#[derive(Debug, Clone)]
pub struct HeatClient {
    config: HeatClientConfig,
    http_client: reqwest::blocking::Client,
    active_experiment: Option<Arc<Mutex<Experiment>>>,
}

/// Type alias for the HeatClient for simplicity
pub type HeatClientState = HeatClient;

impl HeatClient {
    fn new(config: HeatClientConfig) -> HeatClient {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Client should be created.");

        HeatClient {
            config,
            http_client,
            active_experiment: None,
        }
    }

    fn health_check(&self) -> Result<(), reqwest::Error> {
        let url = format!("{}/health", self.get_endpoint());
        self.http_client.get(url).send()?;

        Ok(())
    }

    fn create_and_start_experiment(&self) -> Result<String, HeatSdkError> {
        #[derive(Deserialize)]
        struct ExperimentResponse {
            experiment_id: String,
        }

        let url = format!("{}/experiments", self.get_endpoint());

        // Create a new experiment
        let exp_uuid = self
            .http_client
            .post(url)
            .send()?
            .json::<ExperimentResponse>()?
            .experiment_id;

        // Start the experiment
        self.http_client
            .put(format!(
                "{}/experiments/{}/start",
                self.get_endpoint(),
                exp_uuid
            ))
            .send()?;

        println!("Experiment UUID: {}", exp_uuid);
        Ok(exp_uuid)
    }

    fn request_ws(&self, exp_uuid: String) -> Result<String, HeatSdkError> {
        #[derive(Deserialize)]
        struct WSURLResponse {
            url: String,
        }
        let url = format!("{}/experiments/{}/ws", self.get_endpoint(), exp_uuid);
        let ws_endpoint = self
            .http_client
            .get(url)
            .send()?
            .json::<WSURLResponse>()?
            .url;
        Ok(ws_endpoint)
    }

    /// Create a new HeatClient with the given configuration.
    pub fn create(config: HeatClientConfig) -> Result<HeatClientState, HeatSdkError> {
        let client = HeatClient::new(config);

        // Try to connect to the api, if it fails, return an error
        for i in 0..=client.config.num_retries {
            let res = client.health_check();
            match res {
                Ok(_) => break,
                Err(e) => {
                    if i == client.config.num_retries {
                        return Err(HeatSdkError::ServerTimeoutError(e.to_string()));
                    }
                    println!("Failed to connect to the server. Retrying...");
                }
            }
        }

        Ok(client)
    }

    /// Start a new experiment. This will create a new experiment on the Heat backend and start it.
    pub fn start_experiment(&mut self) -> Result<(), HeatSdkError> {
        let exp_uuid = self.create_and_start_experiment()?;
        let ws_endpoint = self.request_ws(exp_uuid.clone())?;

        let mut ws_client = WebSocketClient::new();
        ws_client.connect(ws_endpoint)?;

        let experiment = Arc::new(Mutex::new(Experiment::new(exp_uuid, ws_client)));
        self.active_experiment = Some(experiment);

        Ok(())
    }

    fn get_endpoint(&self) -> String {
        self.config.endpoint.clone()
    }

    fn request_checkpoint_url(
        &self,
        path: &str,
        access: AccessMode,
    ) -> Result<String, reqwest::Error> {
        let url = format!("{}/checkpoints", self.get_endpoint());

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());
        body.insert(
            "experiment_id",
            self.active_experiment
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .id()
                .clone(),
        );

        let response = match access {
            AccessMode::Read => self.http_client.get(url),
            AccessMode::Write => self.http_client.post(url),
        }
        .json(&body)
        .send()?
        .json::<URLResponse>()?;

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
    ) -> Result<(), HeatSdkError> {
        let url = self.request_checkpoint_url(path, AccessMode::Write)?;

        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        self.upload_checkpoint(&url, checkpoint)?;

        let time_end = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        log::info!("Time to upload checkpoint: {}", time_end - time);
        Ok(())
    }

    /// Load checkpoint data from the Heat API
    pub fn load_checkpoint_data(&self, path: &str) -> Result<Vec<u8>, HeatSdkError> {
        let url = self.request_checkpoint_url(path, AccessMode::Read)?;
        let response = self.download_checkpoint(&url)?;

        Ok(response.to_vec())
    }

    /// Log a message to the active experiment.
    pub fn log_experiment(&mut self, message: String) -> Result<(), HeatSdkError> {
        self.active_experiment
            .as_ref()
            .unwrap()
            .lock()?
            .add_log(message)?;
        Ok(())
    }

    /// End the active experiment. This will close the WebSocket connection and upload the logs to the Heat backend.
    pub fn end_experiment(&mut self) -> Result<(), HeatSdkError> {
        let experiment = self.active_experiment.take().unwrap();
        let experiment = experiment.lock()?;
        let logs = experiment.logs().clone();

        let logs_upload_url = self
            .http_client
            .post(format!(
                "{}/experiments/{}/logs",
                self.get_endpoint(),
                experiment.id()
            ))
            .send()?
            .json::<URLResponse>()?
            .url;

        let logs_string = logs.join("");

        self.http_client
            .put(logs_upload_url)
            .body(logs_string)
            .send()?;

        // End the experiment
        self.http_client
            .put(format!(
                "{}/experiments/{}/end",
                self.get_endpoint(),
                experiment.id()
            ))
            .send()?;

        Ok(())
    }
}
