use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

use burn::tensor::backend::Backend;
use serde::Serialize;

use crate::errors::sdk::HeatSdkError;
use crate::experiment::{Experiment, TempLogStore, WsMessage};
use crate::http::{EndExperimentStatus, HttpClient};
use crate::schemas::{CrateVersionMetadata, PackagedCrateData};
use crate::websocket::WebSocketClient;

/// Credentials to connect to the Heat server
#[derive(Serialize, Debug, Clone)]
pub struct HeatCredentials {
    api_key: String,
}

impl HeatCredentials {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl From<HeatCredentials> for String {
    fn from(val: HeatCredentials) -> Self {
        val.api_key
    }
}

/// Configuration for the HeatClient. Can be created using [HeatClientConfigBuilder], which is created using the [HeatClientConfig::builder] method.
#[derive(Debug, Clone)]
pub struct HeatClientConfig {
    /// The endpoint of the Heat API
    pub endpoint: String,
    /// Heat credential to create a session with the Heat API
    pub credentials: HeatCredentials,
    /// The number of retries to attempt when connecting to the Heat API.
    pub num_retries: u8,
    /// The project ID to create the experiment in.
    pub project_id: String,
}

impl HeatClientConfig {
    /// Create a new [HeatClientConfigBuilder] with the given API key.
    pub fn builder(creds: HeatCredentials, project_id: &str) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder::new(creds, project_id)
    }
}

/// Builder for the HeatClientConfig
pub struct HeatClientConfigBuilder {
    config: HeatClientConfig,
}

impl HeatClientConfigBuilder {
    pub(crate) fn new(creds: HeatCredentials, project_id: &str) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder {
            config: HeatClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                credentials: creds,
                num_retries: 3,
                project_id: project_id.into(),
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
    http_client: HttpClient,
    active_experiment: Arc<RwLock<Option<Experiment>>>,
    active_experiment: Arc<RwLock<Option<Experiment>>>,
}

/// Type alias for the HeatClient for simplicity
pub type HeatClientState = HeatClient;

impl HeatClient {
    fn new(config: HeatClientConfig) -> HeatClient {
        let http_client = HttpClient::new(config.endpoint.clone());

        HeatClient {
            config,
            http_client,
            active_experiment: Arc::new(RwLock::new(None)),
            active_experiment: Arc::new(RwLock::new(None)),
        }
    }

    fn connect(&mut self) -> Result<(), HeatSdkError> {
        self.http_client.login(&self.config.credentials)?;

        Ok(())
    }

    /// Create a new HeatClient with the given configuration.
    pub fn create(config: HeatClientConfig) -> Result<HeatClientState, HeatSdkError> {
        let mut client = HeatClient::new(config);

        // Try to connect to the api, if it fails, return an error
        for i in 0..=client.config.num_retries {
            match client.connect() {
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
    pub fn start_experiment(&mut self, config: &impl Serialize) -> Result<(), HeatSdkError> {
        let exp_uuid = self
            .http_client
            .create_experiment(&self.config.project_id)?;
        self.http_client.start_experiment(&exp_uuid, config)?;

        println!("Experiment UUID: {}", exp_uuid);

        let ws_endpoint = self.http_client.request_websocket_url(&exp_uuid)?;

        let mut ws_client = WebSocketClient::new();
        ws_client.connect(ws_endpoint, self.http_client.get_session_cookie().unwrap())?;

        let exp_log_store = TempLogStore::new(self.http_client.clone(), exp_uuid.clone());

        let experiment = Experiment::new(exp_uuid, ws_client, exp_log_store);
        let mut exp_guard = self
            .active_experiment
            .write()
            .expect("Should be able to lock active_experiment as write.");
        exp_guard.replace(experiment);
        let experiment = Experiment::new(exp_uuid, ws_client, exp_log_store);
        let mut exp_guard = self
            .active_experiment
            .write()
            .expect("Should be able to lock active_experiment as write.");
        exp_guard.replace(experiment);

        Ok(())
    }

    /// Get the sender for the active experiment's WebSocket connection.
    pub fn get_experiment_sender(&self) -> Result<mpsc::Sender<WsMessage>, HeatSdkError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if let Some(w) = active_experiment.as_ref() {
            w.get_ws_sender()
        } else {
            Err(HeatSdkError::ClientError(
                "No active experiment to get sender.".to_string(),
            ))
        }
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if let Some(w) = active_experiment.as_ref() {
            w.get_ws_sender()
        } else {
            Err(HeatSdkError::ClientError(
                "No active experiment to get sender.".to_string(),
            ))
        }
    }

    /// Save checkpoint data to the Heat API.
    pub(crate) fn save_checkpoint_data(
        &self,
        path: &str,
        checkpoint: Vec<u8>,
    ) -> Result<(), HeatSdkError> {
        let active_experiment = self
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let exp_uuid = active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let exp_uuid = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .expect("Experiment should exist.")
            .id()
            .clone();

        let url = self
            .http_client
            .request_checkpoint_save_url(&exp_uuid, path)?;

        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        self.http_client.upload_bytes_to_url(&url, checkpoint)?;

        let time_end = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();

        log::info!("Time to upload checkpoint: {}", time_end - time);
        Ok(())
    }

    /// Load checkpoint data from the Heat API
    pub(crate) fn load_checkpoint_data(&self, path: &str) -> Result<Vec<u8>, HeatSdkError> {
        let active_experiment = self
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let exp_uuid = active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let exp_uuid = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .expect("Experiment should exist.")
            .id()
            .clone();

        let url = self
            .http_client
            .request_checkpoint_load_url(&exp_uuid, path)?;
        let response = self.http_client.download_bytes_from_url(&url)?;

        Ok(response)
    }

    /// Save the final model to the Heat backend.
    pub(crate) fn save_final_model(&self, data: Vec<u8>) -> Result<(), HeatSdkError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if active_experiment.is_none() {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if active_experiment.is_none() {
            return Err(HeatSdkError::ClientError(
                "No active experiment to upload final model.".to_string(),
            ));
        }

        let experiment_id = active_experiment
        let experiment_id = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .expect("Experiment should exist.")
            .id()
            .clone();

        let url = self
            .http_client
            .request_final_model_save_url(&experiment_id)?;
        self.http_client.upload_bytes_to_url(&url, data)?;

        Ok(())
    }

    /// End the active experiment and upload the final model to the Heat backend.
    /// This will close the WebSocket connection and upload the logs to the Heat backend.
    pub fn end_experiment_with_model<B, S>(
        &mut self,
        model: impl burn::module::Module<B>,
    ) -> Result<(), HeatSdkError>
    where
        B: Backend,
        S: burn::record::PrecisionSettings,
    {
        let recorder = crate::record::RemoteRecorder::<S>::final_model(self.clone());
        let res = model.save_file("", &recorder);
        if let Err(e) = res {
            return Err(HeatSdkError::ClientError(e.to_string()));
        }

        self.end_experiment_internal(EndExperimentStatus::Success)
    }

    /// End the active experiment with an error reason.
    /// This will close the WebSocket connection and upload the logs to the Heat backend.
    /// No model will be uploaded.
    pub fn end_experiment_with_error(&mut self, error_reason: String) -> Result<(), HeatSdkError> {
        self.end_experiment_internal(EndExperimentStatus::Fail(error_reason))
    }

    fn end_experiment_internal(
        &mut self,
        end_status: EndExperimentStatus,
    ) -> Result<(), HeatSdkError> {
        let mut active_experiment = self
            .active_experiment
            .write()
            .expect("Should be able to lock active_experiment as write.");
        let mut experiment = active_experiment.take().expect("Experiment should exist.");
        let mut active_experiment = self
            .active_experiment
            .write()
            .expect("Should be able to lock active_experiment as write.");
        let mut experiment = active_experiment.take().expect("Experiment should exist.");

        // Stop the websocket handling thread
        experiment.stop();

        // End the experiment in the backend
        self.http_client
            .end_experiment(experiment.id(), end_status)?;

        Ok(())
    }

    pub fn upload_new_project_version(
        &self,
        root_crate_name: &str,
        crates_data: Vec<PackagedCrateData>,
    ) -> Result<u32, HeatSdkError> {
        let (data, metadata): (Vec<(String, PathBuf)>, Vec<CrateVersionMetadata>) = crates_data
            .into_iter()
            .map(|krate| {
                (
                    (krate.name, krate.path),
                    CrateVersionMetadata {
                        checksum: krate.checksum,
                        metadata: krate.metadata,
                    },
                )
            })
            .unzip();

        let urls = self.http_client.publish_project_version_urls(
            &self.config.project_id,
            root_crate_name,
            metadata,
        )?;

        // assumes that the urls are returned in the same order as the names
        for (crate_name, file_path) in data.into_iter() {
            let url = urls
                .urls
                .get(&crate_name)
                .ok_or(HeatSdkError::ClientError(format!(
                    "No URL found for crate {}",
                    crate_name
                )))?;

            let data = std::fs::read(file_path).map_err(|e| {
                HeatSdkError::ClientError(format!(
                    "Failed to read crate data for {}: {}",
                    crate_name, e
                ))
            })?;

            self.http_client.upload_bytes_to_url(url, data)?;
        }

        Ok(urls.project_version)
    }

    pub fn start_remote_job(
        &self,
        project_version: u32,
        target_package: String,
        command: String,
    ) -> Result<(), HeatSdkError> {
        self.http_client.start_remote_job(
            self.config
                .project_id
                .parse()
                .expect("Project id should be a valid Uuid"),
            project_version,
            target_package,
            command,
        )
    }
}

impl Drop for HeatClient {
    fn drop(&mut self) {
        // if the ref count is 1, then we are the last reference to the client, so we should end the experiment
        if Arc::strong_count(&self.active_experiment) == 1 {
            {
                let active_experiment = self
                    .active_experiment
                    .read()
                    .expect("Should be able to lock active_experiment as read.");
                if active_experiment.is_none() {
                    return;
                }
            }

            self.end_experiment_internal(EndExperimentStatus::Success)
                .expect("Should be able to end the experiment after dropping the last client.");
        if Arc::strong_count(&self.active_experiment) == 1 {
            {
                let active_experiment = self
                    .active_experiment
                    .read()
                    .expect("Should be able to lock active_experiment as read.");
                if active_experiment.is_none() {
                    return;
                }
            }

            self.end_experiment_internal(EndExperimentStatus::Success)
                .expect("Should be able to end the experiment after dropping the last client.");
        }
    }
}
