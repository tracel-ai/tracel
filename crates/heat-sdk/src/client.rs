use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use burn::tensor::backend::Backend;
use reqwest::StatusCode;
use serde::Serialize;

use crate::errors::sdk::HeatSdkError;
use crate::experiment::{Experiment, TempLogStore, WsMessage};
use crate::http::error::HeatHttpError;
use crate::http::{EndExperimentStatus, HttpClient};
use crate::schemas::{
    CrateVersionMetadata, ExperimentPath, HeatCodeMetadata, PackagedCrateData, ProjectPath,
};
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
    /// Whether to use a secure WebSocket connection
    pub wss: bool,
    /// Heat credential to create a session with the Heat API
    pub credentials: HeatCredentials,
    /// The number of retries to attempt when connecting to the Heat API.
    pub num_retries: u8,
    /// The interval to wait between retries in seconds.
    pub retry_interval: u64,
    /// The project ID to create the experiment in.
    pub project_path: ProjectPath,
}

impl HeatClientConfig {
    /// Create a new [HeatClientConfigBuilder] with the given API key.
    pub fn builder(creds: HeatCredentials, project_path: ProjectPath) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder::new(creds, project_path)
    }
}

/// Builder for the HeatClientConfig
pub struct HeatClientConfigBuilder {
    config: HeatClientConfig,
}

impl HeatClientConfigBuilder {
    pub(crate) fn new(
        creds: HeatCredentials,
        project_path: ProjectPath,
    ) -> HeatClientConfigBuilder {
        HeatClientConfigBuilder {
            config: HeatClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                wss: false,
                credentials: creds,
                num_retries: 3,
                retry_interval: 3,
                project_path,
            },
        }
    }

    /// Set the endpoint of the Heat API
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> HeatClientConfigBuilder {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set whether to use a secure WebSocket connection
    /// If this is set to true, the WebSocket connection will use the `wss` protocol instead of `ws`.
    pub fn with_wss(mut self, wss: bool) -> HeatClientConfigBuilder {
        self.config.wss = wss;
        self
    }

    /// Set the number of retries to attempt when connecting to the Heat API
    pub fn with_num_retries(mut self, num_retries: u8) -> HeatClientConfigBuilder {
        self.config.num_retries = num_retries;
        self
    }

    /// Set the interval to wait between retries in seconds
    pub fn with_retry_interval(mut self, retry_interval: u64) -> HeatClientConfigBuilder {
        self.config.retry_interval = retry_interval;
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
}

/// Type alias for the HeatClient for simplicity
pub type HeatClientState = HeatClient;

impl HeatClient {
    fn new(config: HeatClientConfig) -> HeatClient {
        let url = config
            .endpoint
            .parse()
            .expect("Should be able to parse the URL");
        let http_client = HttpClient::new(url, config.wss);

        HeatClient {
            config,
            http_client,
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
                    println!("Failed to connect to the server: {}", e);

                    if i == client.config.num_retries {
                        return Err(HeatSdkError::CreateClientError(
                            "Server timeout".to_string(),
                        ));
                    }

                    if let HeatSdkError::HttpError(HeatHttpError::HttpError(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        msg,
                    )) = e
                    {
                        println!("Invalid API key. Please check your API key and try again.");
                        return Err(HeatSdkError::CreateClientError(format!(
                            "Invalid API key: {msg}"
                        )));
                    }
                    println!(
                        "Failed to connect to the server. Retrying in {} seconds...",
                        client.config.retry_interval
                    );
                    thread::sleep(Duration::from_secs(client.config.retry_interval));
                }
            }
        }

        Ok(client)
    }

    /// Start a new experiment. This will create a new experiment on the Heat backend and start it.
    pub fn start_experiment(&mut self, config: &impl Serialize) -> Result<(), HeatSdkError> {
        let experiment = self
            .http_client
            .create_experiment(
                self.config.project_path.owner_name(),
                self.config.project_path.project_name(),
            )
            .map_err(HeatSdkError::HttpError)?;

        let experiment_path = ExperimentPath::try_from(format!(
            "{}/{}",
            self.config.project_path, experiment.experiment_num
        ))?;

        self.http_client
            .start_experiment(
                self.config.project_path.owner_name(),
                &experiment.project_name,
                experiment.experiment_num,
                config,
            )
            .map_err(HeatSdkError::HttpError)?;

        println!("Experiment num: {}", experiment.experiment_num);

        let ws_endpoint = self.http_client.format_websocket_url(
            self.config.project_path.owner_name(),
            &experiment.project_name,
            experiment.experiment_num,
        );

        let mut ws_client = WebSocketClient::new();
        ws_client.connect(ws_endpoint, self.http_client.get_session_cookie().unwrap())?;

        let exp_log_store = TempLogStore::new(self.http_client.clone(), experiment_path.clone());

        let experiment = Experiment::new(experiment_path, ws_client, exp_log_store);
        let mut exp_guard = self
            .active_experiment
            .write()
            .expect("Should be able to lock active_experiment as write.");
        exp_guard.replace(experiment);

        Ok(())
    }

    /// Get the sender for the active experiment's WebSocket connection.
    pub(crate) fn get_experiment_sender(&self) -> Result<mpsc::Sender<WsMessage>, HeatSdkError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if let Some(w) = active_experiment.as_ref() {
            w.get_ws_sender()
        } else {
            Err(HeatSdkError::UnknownError(
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
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let experiment_path = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .experiment_path()
            .clone();

        let url = self.http_client.request_checkpoint_save_url(
            experiment_path.owner_name(),
            experiment_path.project_name(),
            experiment_path.experiment_num(),
            path,
        )?;

        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Should be able to get time.")
            .as_millis();

        self.http_client.upload_bytes_to_url(&url, checkpoint)?;

        let time_end = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Should be able to get time.")
            .as_millis();

        log::info!("Time to upload checkpoint: {}", time_end - time);
        Ok(())
    }

    /// Load checkpoint data from the Heat API
    pub(crate) fn load_checkpoint_data(&self, path: &str) -> Result<Vec<u8>, HeatSdkError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        let experiment_path = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .experiment_path()
            .clone();

        let url = self.http_client.request_checkpoint_load_url(
            experiment_path.owner_name(),
            experiment_path.project_name(),
            experiment_path.experiment_num(),
            path,
        )?;
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
            return Err(HeatSdkError::UnknownError(
                "No active experiment to upload final model.".to_string(),
            ));
        }

        let experiment_path = active_experiment
            .as_ref()
            .expect("Experiment should exist.")
            .experiment_path()
            .clone();

        let url = self.http_client.request_final_model_save_url(
            experiment_path.owner_name(),
            experiment_path.project_name(),
            experiment_path.experiment_num(),
        )?;
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
            return Err(HeatSdkError::StopExperimentError(e.to_string()));
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
        let experiment_path = experiment.experiment_path().clone();

        // Stop the websocket handling thread
        experiment.stop();

        // End the experiment in the backend
        self.http_client.end_experiment(
            experiment_path.owner_name(),
            experiment_path.project_name(),
            experiment_path.experiment_num(),
            end_status,
        )?;

        Ok(())
    }

    pub fn upload_new_project_version(
        &self,
        target_package_name: &str,
        heat_metadata: HeatCodeMetadata,
        crates_data: Vec<PackagedCrateData>,
        last_commit: &str,
    ) -> Result<String, HeatSdkError> {
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
            self.config.project_path.owner_name(),
            self.config.project_path.project_name(),
            target_package_name,
            heat_metadata,
            metadata,
            last_commit,
        )?;

        for (crate_name, file_path) in data.into_iter() {
            let url = urls
                .urls
                .get(&crate_name)
                .ok_or(HeatSdkError::UnknownError(format!(
                    "No URL found for crate {}",
                    crate_name
                )))?;

            let data = std::fs::read(file_path).map_err(|e| {
                HeatSdkError::FileReadError(format!(
                    "Could not read crate data for crate {}: {}",
                    crate_name, e
                ))
            })?;

            self.http_client.upload_bytes_to_url(url, data)?;
        }

        Ok(urls.project_version)
    }

    /// Checks whether a certain project version exists
    pub fn check_project_version_exists(
        &self,
        project_version: &str,
    ) -> Result<bool, HeatSdkError> {
        let exists = self.http_client.check_project_version_exists(
            self.config.project_path.owner_name(),
            self.config.project_path.project_name(),
            project_version,
        )?;

        Ok(exists)
    }

    pub fn start_remote_job(
        &self,
        runner_group_name: String,
        project_version: &str,
        command: String,
    ) -> Result<(), HeatSdkError> {
        self.http_client.start_remote_job(
            &runner_group_name,
            self.config.project_path.owner_name(),
            self.config.project_path.project_name(),
            project_version,
            command,
        )?;

        Ok(())
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
        }
    }
}
