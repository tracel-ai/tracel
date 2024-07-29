use std::collections::HashMap;

use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Serialize;
use uuid::Uuid;

use crate::{client::HeatCredentials, error::HeatSdkError, http::schemas::StartExperimentSchema};

use super::schemas::{
    CodeUploadParamsSchema, CodeUploadUrlsSchema, CreateExperimentResponseSchema,
    EndExperimentSchema, RunnerJobCommand, RunnerQueueJobParamsSchema, URLSchema,
};

pub enum EndExperimentStatus {
    Success,
    Fail(String),
}

impl From<reqwest::Error> for HeatSdkError {
    fn from(error: reqwest::Error) -> Self {
        match error.status() {
            Some(status) => match status {
                reqwest::StatusCode::REQUEST_TIMEOUT => {
                    HeatSdkError::ServerTimeoutError(error.to_string())
                }
                _ => HeatSdkError::ServerError(status.to_string()),
            },
            None => HeatSdkError::ServerError(error.to_string()),
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
    pub fn health_check(&self) -> Result<(), HeatSdkError> {
        let url = format!("{}/health", self.base_url);
        self.http_client.get(url).send()?.error_for_status()?;
        Ok(())
    }

    /// Get the session cookie if it exists.
    pub fn get_session_cookie(&self) -> Option<&String> {
        self.session_cookie.as_ref()
    }

    /// Log in to the Heat server with the given credentials.
    pub fn login(&mut self, credentials: &HeatCredentials) -> Result<(), HeatSdkError> {
        let url = format!("{}/login/api-key", self.base_url);
        let res = self
            .http_client
            .post(url)
            .form::<HeatCredentials>(credentials)
            .send()?;

        // store session cookie
        if res.status().is_success() {
            let cookie_header = res.headers().get(SET_COOKIE);
            if let Some(cookie) = cookie_header {
                let cookie_str = cookie
                    .to_str()
                    .expect("Session cookie should be convert to str");
                self.session_cookie = Some(cookie_str.to_string());
            } else {
                return Err(HeatSdkError::ClientError(
                    "Cannot connect to Heat server, bad session ID.".to_string(),
                ));
            }
        } else {
            let error_message: String = format!("Cannot connect to Heat server({:?})", res.text()?);
            return Err(HeatSdkError::ClientError(error_message));
        }

        Ok(())
    }

    /// Request a WebSocket URL for the given experiment.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_websocket_url(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/ws", self.base_url.clone(), exp_uuid);
        let ws_endpoint = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;
        Ok(ws_endpoint)
    }

    /// Create a new experiment for the given project.
    ///
    /// The client must be logged in before calling this method.
    pub fn create_experiment(&self, project_id: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/projects/{}/experiments", self.base_url, project_id);

        // Create a new experiment
        let exp_uuid = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<CreateExperimentResponseSchema>()?
            .experiment_id;

        Ok(exp_uuid)
    }

    /// Start the experiment with the given configuration.
    ///
    /// The client must be logged in before calling this method.
    pub fn start_experiment(
        &self,
        exp_uuid: &str,
        config: &impl Serialize,
    ) -> Result<(), HeatSdkError> {
        self.validate_session_cookie()?;

        let json = StartExperimentSchema {
            config: serde_json::to_value(config).unwrap(),
        };

        // Start the experiment
        self.http_client
            .put(format!("{}/experiments/{}/start", self.base_url, exp_uuid))
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&json)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    /// End the experiment with the given status.
    ///
    /// The client must be logged in before calling this method.
    pub fn end_experiment(
        &self,
        exp_uuid: &str,
        end_status: EndExperimentStatus,
    ) -> Result<(), HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/end", self.base_url, exp_uuid);

        let end_status: EndExperimentSchema = match end_status {
            EndExperimentStatus::Success => EndExperimentSchema::Success,
            EndExperimentStatus::Fail(reason) => EndExperimentSchema::Fail(reason),
        };

        self.http_client
            .put(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&end_status)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    /// Save the checkpoint data to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_save_url(
        &self,
        exp_uuid: &str,
        path: &str,
    ) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url: String = format!("{}/checkpoints", self.base_url);

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());
        body.insert("experiment_id", exp_uuid.to_string());

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .error_for_status()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(save_url)
    }

    /// Request a URL to load the checkpoint data from the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_load_url(
        &self,
        exp_uuid: &str,
        path: &str,
    ) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url: String = format!("{}/checkpoints/load", self.base_url);

        let mut body = HashMap::new();
        body.insert("file_path", path.to_string());
        body.insert("experiment_id", exp_uuid.to_string());

        let load_url = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .error_for_status()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(load_url)
    }

    /// Request a URL to save the final model to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_final_model_save_url(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/save_model", self.base_url, exp_uuid);

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;

        Ok(save_url)
    }

    /// Request a URL to upload logs to the Heat server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_logs_upload_url(&self, exp_uuid: &str) -> Result<String, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/experiments/{}/logs", self.base_url, exp_uuid);

        let logs_upload_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .error_for_status()?
            .json::<URLSchema>()?
            .url;

        Ok(logs_upload_url)
    }

    /// Generic method to upload bytes to the given URL.
    pub fn upload_bytes_to_url(&self, url: &str, bytes: Vec<u8>) -> Result<(), HeatSdkError> {
        self.http_client
            .put(url)
            .body(bytes)
            .send()?
            .error_for_status()?;

        Ok(())
    }

    /// Generic method to download bytes from the given URL.
    pub fn download_bytes_from_url(&self, url: &str) -> Result<Vec<u8>, HeatSdkError> {
        let data = self
            .http_client
            .get(url)
            .send()?
            .error_for_status()?
            .bytes()?
            .to_vec();

        Ok(data)
    }

    fn validate_session_cookie(&self) -> Result<(), HeatSdkError> {
        if self.session_cookie.is_none() {
            return Err(HeatSdkError::ClientError(
                "Cannot connect to Heat server, no session ID.".to_string(),
            ));
        }
        Ok(())
    }

    pub fn request_code_upload_urls(
        &self,
        project_id: &str,
        crate_names: Vec<String>,
    ) -> Result<CodeUploadUrlsSchema, HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/code/upload", self.base_url);

        let upload_urls = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&CodeUploadParamsSchema {
                project_id: project_id.to_string(),
                crate_names,
            })
            .send()?
            .error_for_status()?
            .json::<CodeUploadUrlsSchema>()?;

        Ok(upload_urls)
    }

    pub fn start_remote_job(
        &self,
        project_id: Uuid,
        project_version: u32,
        command: String,
    ) -> Result<(), HeatSdkError> {
        self.validate_session_cookie()?;

        let url = format!("{}/runner/queue", self.base_url);

        let body = RunnerQueueJobParamsSchema {
            project_id,
            project_version,
            command: RunnerJobCommand { command },
        };

        self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .error_for_status()?;

        Ok(())
    }
}
