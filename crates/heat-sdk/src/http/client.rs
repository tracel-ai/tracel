use reqwest::header::{COOKIE, SET_COOKIE};
use reqwest::Url;
use serde::Serialize;

use crate::http::error::HeatHttpError;
use crate::schemas::HeatCodeMetadata;
use crate::{
    client::HeatCredentials,
    http::schemas::StartExperimentSchema,
    schemas::{CrateVersionMetadata, Experiment},
};

use super::schemas::{
    CodeUploadParamsSchema, CodeUploadUrlsSchema, CreateExperimentResponseSchema,
    EndExperimentSchema, RunnerQueueJobParamsSchema, URLSchema,
};

pub enum EndExperimentStatus {
    Success,
    Fail(String),
}

impl From<reqwest::Error> for HeatHttpError {
    fn from(error: reqwest::Error) -> Self {
        match error.status() {
            Some(status) => HeatHttpError::HttpError(status, error.to_string()),
            None => HeatHttpError::UnknownError(error.to_string()),
        }
    }
}

trait ResponseExt {
    fn map_to_heat_err(self) -> Result<reqwest::blocking::Response, HeatHttpError>;
}

impl ResponseExt for reqwest::blocking::Response {
    fn map_to_heat_err(self) -> Result<reqwest::blocking::Response, HeatHttpError> {
        if self.status().is_success() {
            Ok(self)
        } else {
            Err(HeatHttpError::HttpError(self.status(), self.text()?))
        }
    }
}

/// A client for making HTTP requests to the Heat API.
///
/// The client can be used to interact with the Heat server, such as creating and starting experiments, saving and loading checkpoints, and uploading logs.
#[derive(Debug, Clone)]
pub struct HttpClient {
    http_client: reqwest::blocking::Client,
    base_url: String,
    session_cookie: Option<String>,
}

impl HttpClient {
    /// Create a new HttpClient with the given base URL and API key.
    pub fn new(base_url: String) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            session_cookie: None,
        }
    }

    /// Create a new HttpClient with the given base URL, API key, and session cookie.
    #[allow(dead_code)]
    pub fn with_session_cookie(base_url: String, session_cookie: String) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            session_cookie: Some(session_cookie),
        }
    }

    /// Check if the Heat server is reachable.
    #[allow(dead_code)]
    pub fn health_check(&self) -> Result<(), HeatHttpError> {
        let url = format!("{}/health", self.base_url);
        self.http_client.get(url).send()?.map_to_heat_err()?;
        Ok(())
    }

    /// Get the session cookie if it exists.
    pub fn get_session_cookie(&self) -> Option<&String> {
        self.session_cookie.as_ref()
    }

    /// Log in to the Heat server with the given credentials.
    pub fn login(&mut self, credentials: &HeatCredentials) -> Result<(), HeatHttpError> {
        let url = format!("{}/login/api-key", self.base_url);
        let res = self
            .http_client
            .post(url)
            .form::<HeatCredentials>(credentials)
            .send()?;

        let status = res.status();

        // store session cookie
        if status.is_success() {
            let cookie_header = res.headers().get(SET_COOKIE);
            if let Some(cookie) = cookie_header {
                let cookie_str = cookie
                    .to_str()
                    .expect("Session cookie should be able to convert to str");
                self.session_cookie = Some(cookie_str.to_string());
            } else {
                return Err(HeatHttpError::BadSessionId);
            }
        } else {
            let error_message: String = format!("Cannot connect to Heat server({:?})", res.text()?);
            return Err(HeatHttpError::HttpError(status, error_message));
        }

        Ok(())
    }

    /// Formats a WebSocket URL for the given experiment.
    pub fn format_websocket_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> String {
        let mut url: Url = self
            .base_url
            .parse()
            .expect("Should be able to parse base url");
        url.set_scheme("wss")
            .expect("Should be able to set ws scheme");
        format!(
            "{}/projects/{}/{}/experiments/{}/ws",
            url, owner_name, project_name, exp_num
        )
    }

    /// Create a new experiment for the given project.
    ///
    /// The client must be logged in before calling this method.
    pub fn create_experiment(
        &self,
        owner_name: &str,
        project_name: &str,
    ) -> Result<Experiment, HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/experiments",
            self.base_url, owner_name, project_name
        );

        // Create a new experiment
        let experiment_response = self
            .http_client
            .post(url)
            .json(&serde_json::json!({}))
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_heat_err()?
            .json::<CreateExperimentResponseSchema>()?;

        let experiment = Experiment {
            experiment_num: experiment_response.experiment_num,
            project_name: project_name.to_string(),
            status: experiment_response.status,
            description: experiment_response.description,
            config: experiment_response.config,
            created_by: experiment_response.created_by,
            created_at: experiment_response.created_at,
        };

        Ok(experiment)
    }

    /// Start the experiment with the given configuration.
    ///
    /// The client must be logged in before calling this method.
    pub fn start_experiment(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        config: &impl Serialize,
    ) -> Result<(), HeatHttpError> {
        self.validate_session_cookie()?;

        let json = StartExperimentSchema {
            config: serde_json::to_value(config).unwrap(),
        };

        // Start the experiment
        self.http_client
            .put(format!(
                "{}/projects/{}/{}/experiments/{}/start",
                self.base_url, owner_name, project_name, exp_num
            ))
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&json)
            .send()?
            .map_to_heat_err()?;

        Ok(())
    }

    /// End the experiment with the given status.
    ///
    /// The client must be logged in before calling this method.
    pub fn end_experiment(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        end_status: EndExperimentStatus,
    ) -> Result<(), HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/experiments/{}/end",
            self.base_url, owner_name, project_name, exp_num
        );

        let end_status: EndExperimentSchema = match end_status {
            EndExperimentStatus::Success => EndExperimentSchema::Success,
            EndExperimentStatus::Fail(reason) => EndExperimentSchema::Fail(reason),
        };

        self.http_client
            .put(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&end_status)
            .send()?
            .map_to_heat_err()?;

        Ok(())
    }

    /// Save the checkpoint data to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_save_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, HeatHttpError> {
        self.validate_session_cookie()?;

        let url: String = format!(
            "{}/projects/{}/{}/experiments/{}/checkpoints/{}",
            self.base_url, owner_name, project_name, exp_num, file_name
        );

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_heat_err()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(save_url)
    }

    /// Request a URL to load the checkpoint data from the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_load_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, HeatHttpError> {
        self.validate_session_cookie()?;

        let url: String = format!(
            "{}/projects/{}/{}/experiments/{}/checkpoints/{}",
            self.base_url, owner_name, project_name, exp_num, file_name
        );

        let load_url = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_heat_err()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(load_url)
    }

    /// Request a URL to save the final model to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_final_model_save_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> Result<String, HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/experiments/{}/save_model",
            self.base_url, owner_name, project_name, exp_num
        );

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_heat_err()?
            .json::<URLSchema>()?
            .url;

        Ok(save_url)
    }

    /// Request a URL to upload logs to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_logs_upload_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> Result<String, HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/experiments/{}/logs",
            self.base_url, owner_name, project_name, exp_num
        );

        let logs_upload_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_heat_err()?
            .json::<URLSchema>()?
            .url;

        Ok(logs_upload_url)
    }

    /// Generic method to upload bytes to the given URL.
    pub fn upload_bytes_to_url(&self, url: &str, bytes: Vec<u8>) -> Result<(), HeatHttpError> {
        self.http_client
            .put(url)
            .body(bytes)
            .send()?
            .map_to_heat_err()?;

        Ok(())
    }

    /// Generic method to download bytes from the given URL.
    pub fn download_bytes_from_url(&self, url: &str) -> Result<Vec<u8>, HeatHttpError> {
        let data = self
            .http_client
            .get(url)
            .send()?
            .map_to_heat_err()?
            .bytes()?
            .to_vec();

        Ok(data)
    }

    fn validate_session_cookie(&self) -> Result<(), HeatHttpError> {
        if self.session_cookie.is_none() {
            return Err(HeatHttpError::BadSessionId);
        }
        Ok(())
    }

    pub fn publish_project_version_urls(
        &self,
        owner_name: &str,
        project_name: &str,
        target_package_name: &str,
        heat_metadata: HeatCodeMetadata,
        crates_metadata: Vec<CrateVersionMetadata>,
    ) -> Result<CodeUploadUrlsSchema, HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/code/upload",
            self.base_url, owner_name, project_name
        );

        let response = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&CodeUploadParamsSchema {
                target_package_name: target_package_name.to_string(),
                heat_metadata,
                crates: crates_metadata,
            })
            .send()?
            .map_to_heat_err()?;

        let upload_urls = response.json::<CodeUploadUrlsSchema>()?;
        Ok(upload_urls)
    }

    pub fn start_remote_job(
        &self,
        runner_group_name: &str,
        owner_name: &str,
        project_name: &str,
        project_version: u32,
        command: String,
    ) -> Result<(), HeatHttpError> {
        self.validate_session_cookie()?;

        let url = format!(
            "{}/projects/{}/{}/jobs/queue",
            self.base_url, owner_name, project_name
        );

        let body = RunnerQueueJobParamsSchema {
            runner_group_name: runner_group_name.to_string(),
            project_version,
            command,
        };

        self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .map_to_heat_err()?;

        Ok(())
    }
}
