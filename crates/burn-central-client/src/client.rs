use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use burn::tensor::backend::Backend;
use reqwest::StatusCode;
use serde::Serialize;

use crate::error::BurnCentralClientError;
use crate::experiment::{Experiment, TempLogStore, WsMessage};
use crate::http::error::BurnCentralHttpError;
use crate::http::{EndExperimentStatus, HttpClient};
use crate::schemas::{
    BurnCentralCodeMetadata, CrateVersionMetadata, ExperimentPath, PackagedCrateData, ProjectPath,
    User,
};
use crate::websocket::WebSocketClient;

/// Credentials to connect to the Burn Central server
#[derive(Serialize, Debug, Clone)]
pub struct BurnCentralCredentials {
    api_key: String,
}

impl BurnCentralCredentials {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

impl From<BurnCentralCredentials> for String {
    fn from(val: BurnCentralCredentials) -> Self {
        val.api_key
    }
}

/// Configuration for the BurnCentralClient. Can be created using [BurnCentralClientConfigBuilder], which is created using the [BurnCentralClientConfig::builder] method.
#[derive(Debug, Clone)]
pub struct BurnCentralClientConfig {
    /// The endpoint of the Burn Central API
    pub endpoint: String,
    /// Burn Central credential to create a session with the Burn Central API
    pub credentials: BurnCentralCredentials,
    /// The number of retries to attempt when connecting to the Burn Central API.
    pub num_retries: u8,
    /// The interval to wait between retries in seconds.
    pub retry_interval: u64,
    /// The project path to create the experiment in.
    pub project_path: Option<ProjectPath>,
}

impl BurnCentralClientConfig {
    /// Create a new [BurnCentralClientConfigBuilder] with the given API key.
    pub fn builder(creds: BurnCentralCredentials) -> BurnCentralClientConfigBuilder {
        BurnCentralClientConfigBuilder::new(creds)
    }
}

/// Builder for the BurnCentralClientConfig
pub struct BurnCentralClientConfigBuilder {
    config: BurnCentralClientConfig,
}

impl BurnCentralClientConfigBuilder {
    pub(crate) fn new(creds: BurnCentralCredentials) -> BurnCentralClientConfigBuilder {
        BurnCentralClientConfigBuilder {
            config: BurnCentralClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                credentials: creds,
                num_retries: 3,
                retry_interval: 3,
                project_path: None,
            },
        }
    }

    /// Set the endpoint of the Burn Central API
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> BurnCentralClientConfigBuilder {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set the number of retries to attempt when connecting to the Burn Central API
    pub fn with_num_retries(mut self, num_retries: u8) -> BurnCentralClientConfigBuilder {
        self.config.num_retries = num_retries;
        self
    }

    /// Set the interval to wait between retries in seconds
    pub fn with_retry_interval(mut self, retry_interval: u64) -> BurnCentralClientConfigBuilder {
        self.config.retry_interval = retry_interval;
        self
    }

    /// Set the project to create the experiment in
    pub fn with_project(mut self, project_path: ProjectPath) -> BurnCentralClientConfigBuilder {
        self.config.project_path = Some(project_path);
        self
    }

    /// Build the BurnCentralClientConfig
    pub fn build(self) -> BurnCentralClientConfig {
        self.config
    }
}

/// The BurnCentralClient is used to interact with the Burn Central API. It is required for all interactions with the Burn Central API.
#[derive(Debug, Clone)]
pub struct BurnCentralClient {
    config: BurnCentralClientConfig,
    http_client: HttpClient,
    active_experiment: Arc<RwLock<Option<Experiment>>>,
    current_project: Arc<RwLock<Option<ProjectPath>>>,
}

/// Type alias for the BurnCentralClient for simplicity
pub type BurnCentralClientState = BurnCentralClient;

impl BurnCentralClient {
    fn new(config: BurnCentralClientConfig) -> BurnCentralClient {
        let url = config
            .endpoint
            .parse()
            .expect("Should be able to parse the URL");
        let http_client = HttpClient::new(url);

        BurnCentralClient {
            config,
            http_client,
            active_experiment: Default::default(),
            current_project: Default::default(),
        }
    }

    fn connect(&mut self) -> Result<(), BurnCentralClientError> {
        self.http_client.login(&self.config.credentials)?;

        Ok(())
    }

    fn is_experiment_running(&self) -> bool {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        active_experiment.is_some()
    }

    #[allow(dead_code)]
    fn is_project_set(&self) -> bool {
        let current_project = self
            .current_project
            .read()
            .expect("Should be able to lock current_project as read.");
        current_project.is_some()
    }

    fn get_project_path(&self) -> Option<ProjectPath> {
        let current_project = self
            .current_project
            .read()
            .expect("Should be able to lock current_project as read.");
        current_project.clone()
    }

    pub fn set_project(&mut self, project_path: ProjectPath) -> Result<(), BurnCentralClientError> {
        if self.is_experiment_running() {
            return Err(BurnCentralClientError::SetProjectError(
                "Cannot set project while an experiment is running.".to_string(),
            ));
        }
        let _ = self
            .http_client
            .get_project(project_path.owner_name(), project_path.project_name())?;

        let mut current_project = self
            .current_project
            .write()
            .expect("Should be able to lock current_project as write.");
        *current_project = Some(project_path.clone());

        Ok(())
    }

    /// Create a new BurnCentralClient with the given configuration.
    pub fn create(
        config: BurnCentralClientConfig,
    ) -> Result<BurnCentralClientState, BurnCentralClientError> {
        let mut client = BurnCentralClient::new(config);

        // Try to connect to the api, if it fails, return an error
        for i in 0..=client.config.num_retries {
            match client.connect() {
                Ok(_) => break,
                Err(e) => {
                    // println!("Failed to connect to the server: {}", e);

                    if i == client.config.num_retries {
                        return Err(BurnCentralClientError::ServerConnectionError(
                            "Server timeout".to_string(),
                        ));
                    }

                    if let BurnCentralClientError::HttpError(BurnCentralHttpError::HttpError {
                        status: StatusCode::UNPROCESSABLE_ENTITY,
                        body: msg,
                    }) = e
                    {
                        // println!("Invalid API key. Please check your API key and try again.");
                        return Err(BurnCentralClientError::InvalidCredentialsError(format!(
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

    pub fn get_current_user(&self) -> Result<User, BurnCentralClientError> {
        self.http_client
            .get_current_user()
            .map_err(BurnCentralClientError::HttpError)
            .map(|user| User {
                username: user.username,
                email: user.email,
            })
    }

    /// Start a new experiment. This will create a new experiment on the Burn Central backend and start it.
    pub fn start_experiment(
        &mut self,
        config: &impl Serialize,
    ) -> Result<(), BurnCentralClientError> {
        let Some(project_path) = self.get_project_path() else {
            return Err(BurnCentralClientError::StartExperimentError(
                "No project set. Please set a project before starting an experiment.".to_string(),
            ));
        };

        let experiment = self
            .http_client
            .create_experiment(project_path.owner_name(), project_path.project_name())
            .map_err(BurnCentralClientError::HttpError)?;

        let experiment_path =
            ExperimentPath::try_from(format!("{}/{}", project_path, experiment.experiment_num))?;

        self.http_client
            .start_experiment(
                project_path.owner_name(),
                &experiment.project_name,
                experiment.experiment_num,
                config,
            )
            .map_err(BurnCentralClientError::HttpError)?;

        println!("Experiment num: {}", experiment.experiment_num);

        let ws_endpoint = self.http_client.format_websocket_url(
            project_path.owner_name(),
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
    pub(crate) fn get_experiment_sender(
        &self,
    ) -> Result<mpsc::Sender<WsMessage>, BurnCentralClientError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");
        if let Some(w) = active_experiment.as_ref() {
            w.get_ws_sender()
        } else {
            Err(BurnCentralClientError::UnknownError(
                "No active experiment to get sender.".to_string(),
            ))
        }
    }

    /// Save checkpoint data to the Burn Central API.
    pub(crate) fn save_checkpoint_data(
        &self,
        path: &str,
        checkpoint: Vec<u8>,
    ) -> Result<(), BurnCentralClientError> {
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

    /// Load checkpoint data from the Burn Central API
    pub(crate) fn load_checkpoint_data(
        &self,
        path: &str,
    ) -> Result<Vec<u8>, BurnCentralClientError> {
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

    /// Save the final model to the Burn Central backend.
    pub(crate) fn save_final_model(&self, data: Vec<u8>) -> Result<(), BurnCentralClientError> {
        let active_experiment = self
            .active_experiment
            .read()
            .expect("Should be able to lock active_experiment as read.");

        if active_experiment.is_none() {
            return Err(BurnCentralClientError::UnknownError(
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

    /// End the active experiment and upload the final model to the Burn Central backend.
    /// This will close the WebSocket connection and upload the logs to the Burn Central backend.
    pub fn end_experiment_with_model<B, S>(
        &mut self,
        model: impl burn::module::Module<B>,
    ) -> Result<(), BurnCentralClientError>
    where
        B: Backend,
        S: burn::record::PrecisionSettings,
    {
        let recorder = crate::record::RemoteRecorder::<S>::final_model(self.clone());
        let res = model.save_file("", &recorder);
        if let Err(e) = res {
            return Err(BurnCentralClientError::StopExperimentError(e.to_string()));
        }

        self.end_experiment_internal(EndExperimentStatus::Success)
    }

    /// End the active experiment with an error reason.
    /// This will close the WebSocket connection and upload the logs to the Burn Central backend.
    /// No model will be uploaded.
    pub fn end_experiment_with_error(
        &mut self,
        error_reason: String,
    ) -> Result<(), BurnCentralClientError> {
        self.end_experiment_internal(EndExperimentStatus::Fail(error_reason))
    }

    fn end_experiment_internal(
        &mut self,
        end_status: EndExperimentStatus,
    ) -> Result<(), BurnCentralClientError> {
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

    pub fn create_project(
        &self,
        namespace_name: &str,
        project_name: &str,
        description: Option<&str>,
    ) -> Result<ProjectPath, BurnCentralClientError> {
        self.http_client.create_project(
            namespace_name,
            project_name,
            description,
        ).map_err(|e| {
            BurnCentralClientError::CreateProjectError(format!(
                "Failed to create project: {}",
                e
            ))
        })?;

        let new_project_path = ProjectPath::new(namespace_name.to_string(), project_name.to_string());
        Ok(new_project_path)
    }

    pub fn upload_new_project_version(
        &self,
        target_package_name: &str,
        code_metadata: BurnCentralCodeMetadata,
        crates_data: Vec<PackagedCrateData>,
        last_commit: &str,
    ) -> Result<String, BurnCentralClientError> {
        let Some(project_path) = self.get_project_path() else {
            return Err(BurnCentralClientError::UploadProjectVersionError(
                "No project set. Please set a project before uploading a new version.".to_string(),
            ));
        };

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
            project_path.owner_name(),
            project_path.project_name(),
            target_package_name,
            code_metadata,
            metadata,
            last_commit,
        )?;

        for (crate_name, file_path) in data.into_iter() {
            let url = urls
                .urls
                .get(&crate_name)
                .ok_or(BurnCentralClientError::UnknownError(format!(
                    "No URL found for crate {}",
                    crate_name
                )))?;

            let data = std::fs::read(file_path).map_err(|e| {
                BurnCentralClientError::FileReadError(format!(
                    "Could not read crate data for crate {}: {}",
                    crate_name, e
                ))
            })?;

            self.http_client.upload_bytes_to_url(url, data)?;
        }

        Ok(urls.project_version)
    }

    /// Start a remote job on the Burn Central backend.
    pub fn start_remote_job(
        &self,
        runner_group_name: String,
        project_version: &str,
        command: String,
    ) -> Result<(), BurnCentralClientError> {
        let Some(project_path) = self.get_project_path() else {
            return Err(BurnCentralClientError::StartRemoteJobError(
                "No project set. Please set a project before starting a remote job.".to_string(),
            ));
        };

        if !self.http_client.check_project_version_exists(
            project_path.owner_name(),
            project_path.project_name(),
            project_version,
        )? {
            return Err(BurnCentralClientError::StartRemoteJobError(format!(
                "Project version `{}` does not exist. Please upload your code using the `package` command then you can run your code remotely with that version.",
                project_version
            )));
        }

        self.http_client.start_remote_job(
            &runner_group_name,
            project_path.owner_name(),
            project_path.project_name(),
            project_version,
            command,
        )?;

        Ok(())
    }
}

impl Drop for BurnCentralClient {
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
