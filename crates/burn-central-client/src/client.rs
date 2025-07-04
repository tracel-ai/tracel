use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use burn::tensor::backend::Backend;
use reqwest::StatusCode;
use serde::Serialize;

use crate::error::BurnCentralClientError;
use crate::experiment::{Experiment, ExperimentMessage, TempLogStore};
use crate::http::error::BurnCentralHttpError;
use crate::http::{EndExperimentStatus, HttpClient};
use crate::schemas::{
    BurnCentralCodeMetadata, CrateVersionMetadata, ExperimentPath, PackagedCrateData, Project,
    ProjectPath, User,
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

/// Configuration for the BurnCentralClient. Can be created using [BurnCentralClientBuilder], which is created using the [BurnCentralClientConfig::builder] method.
#[derive(Debug, Clone)]
pub struct BurnCentralClientConfig {
    /// The endpoint of the Burn Central API
    pub endpoint: String,
    /// Burn Central credential to create a session with the Burn Central API
    pub credentials: BurnCentralCredentials,
    /// The project path to create the experiment in.
    pub project_path: Option<ProjectPath>,
}

impl BurnCentralClientConfig {
    /// Create a new [BurnCentralClientBuilder] with the given API key.
    pub fn builder(creds: BurnCentralCredentials) -> BurnCentralClientBuilder {
        BurnCentralClientBuilder::new(creds)
    }
}

/// Builder for BurnCentralClient.
pub struct BurnCentralClientBuilder {
    config: BurnCentralClientConfig,
}

impl BurnCentralClientBuilder {
    pub(crate) fn new(creds: BurnCentralCredentials) -> Self {
        BurnCentralClientBuilder {
            config: BurnCentralClientConfig {
                endpoint: "http://127.0.0.1:9001".into(),
                credentials: creds,
                project_path: None,
            },
        }
    }

    /// Set the endpoint of the Burn Central API
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.config.endpoint = endpoint.into();
        self
    }

    /// Set the project to create the experiment in
    pub fn with_project(mut self, project_path: ProjectPath) -> Self {
        self.config.project_path = Some(project_path);
        self
    }

    /// Build the BurnCentralClientConfig
    pub fn build(self) -> Result<BurnCentralClient, BurnCentralClientError> {
        BurnCentralClient::new(self.config)
    }
}

/// The BurnCentralClient is used to interact with the Burn Central API. It is required for all interactions with the Burn Central API.
#[derive(Debug, Clone)]
pub struct BurnCentralClient {
    http_client: HttpClient,
}

/// Type alias for the BurnCentralClient for simplicity
pub type BurnCentralClientState = BurnCentralClient;

impl BurnCentralClient {
    pub fn builder(creds: BurnCentralCredentials) -> BurnCentralClientBuilder {
        BurnCentralClientBuilder::new(creds)
    }

    fn new(config: BurnCentralClientConfig) -> Result<Self, BurnCentralClientError> {
        let url = config
            .endpoint
            .parse()
            .expect("Should be able to parse the URL");
        let http_client = HttpClient::new(url, &config.credentials).map_err(|e| {
            if let BurnCentralHttpError::HttpError {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                body: msg,
            } = e
            {
                return BurnCentralClientError::InvalidCredentialsError(format!(
                    "Invalid API key: {msg}"
                ));
            }

            BurnCentralClientError::ServerConnectionError("Server timeout".to_string())
        })?;

        Ok(BurnCentralClient { http_client })
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
        namespace: &str,
        project_name: &str,
        config: &impl Serialize,
    ) -> Result<Experiment, BurnCentralClientError> {
        let experiment = self
            .http_client
            .create_experiment(namespace, project_name)
            .map_err(BurnCentralClientError::HttpError)?;

        let experiment_path = ExperimentPath::try_from(format!(
            "{}/{}/{}",
            namespace, project_name, experiment.experiment_num
        ))?;

        self.http_client
            .start_experiment(
                namespace,
                &experiment.project_name,
                experiment.experiment_num,
                config,
            )
            .map_err(BurnCentralClientError::HttpError)?;

        println!("Experiment num: {}", experiment.experiment_num);

        Experiment::new(self.http_client.clone(), experiment_path)
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

    // /// End the active experiment and upload the final model to the Burn Central backend.
    // /// This will close the WebSocket connection and upload the logs to the Burn Central backend.
    // pub fn end_experiment_with_model<B, S>(
    //     &mut self,
    //     model: impl burn::module::Module<B>,
    // ) -> Result<(), BurnCentralClientError>
    // where
    //     B: Backend,
    //     S: burn::record::PrecisionSettings,
    // {
    //     let recorder = crate::record::RemoteRecorder::<S>::final_model(self.clone());
    //     let res = model.save_file("", &recorder);
    //     if let Err(e) = res {
    //         return Err(BurnCentralClientError::StopExperimentError(e.to_string()));
    //     }
    //
    //     self.end_experiment_internal(EndExperimentStatus::Success)
    // }

    // /// End the active experiment with an error reason.
    // /// This will close the WebSocket connection and upload the logs to the Burn Central backend.
    // /// No model will be uploaded.
    // pub fn end_experiment_with_error(
    //     &mut self,
    //     error_reason: String,
    // ) -> Result<(), BurnCentralClientError> {
    //     self.end_experiment_internal(EndExperimentStatus::Fail(error_reason))
    // }

    pub fn find_project(
        &self,
        namespace_name: &str,
        project_name: &str,
    ) -> Result<Option<Project>, BurnCentralClientError> {
        let project = self
            .http_client
            .get_project(namespace_name, project_name)
            .map(Some)
            .or_else(|e| {
                if let BurnCentralHttpError::HttpError {
                    status: StatusCode::NOT_FOUND,
                    ..
                } = e
                {
                    Ok(None)
                } else {
                    Err(e)
                }
            })
            .map_err(|e| BurnCentralClientError::GetProjectError(format!("{project_name}: {e}")))?
            .map(|project_schema| Project {
                project_name: project_schema.project_name,
                namespace_name: project_schema.namespace_name,
                namespace_type: project_schema.namespace_type,
                description: project_schema.description,
                created_by: project_schema.created_by,
                created_at: project_schema.created_at,
                visibility: project_schema.visibility,
            });
        Ok(project)
    }

    pub fn create_project(
        &self,
        namespace_name: &str,
        project_name: &str,
        description: Option<&str>,
    ) -> Result<ProjectPath, BurnCentralClientError> {
        self.http_client
            .create_project(namespace_name, project_name, description)
            .map_err(|e| {
                BurnCentralClientError::CreateProjectError(format!("Failed to create project: {e}"))
            })?;

        let new_project_path =
            ProjectPath::new(namespace_name.to_string(), project_name.to_string());
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
                    "No URL found for crate {crate_name}"
                )))?;

            let data = std::fs::read(file_path).map_err(|e| {
                BurnCentralClientError::FileReadError(format!(
                    "Could not read crate data for crate {crate_name}: {e}"
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
                "Project version `{project_version}` does not exist. Please upload your code using the `package` command then you can run your code remotely with that version."
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

// impl Drop for BurnCentralClient {
//     fn drop(&mut self) {
//         // if the ref count is 1, then we are the last reference to the client, so we should end the experiment
//         if Arc::strong_count(&self.active_experiment) == 1 {
//             {
//                 let active_experiment = self
//                     .active_experiment
//                     .read()
//                     .expect("Should be able to lock active_experiment as read.");
//                 if active_experiment.is_none() {
//                     return;
//                 }
//             }
//
//             self.end_experiment_internal(EndExperimentStatus::Success)
//                 .expect("Should be able to end the experiment after dropping the last client.");
//         }
//     }
// }
