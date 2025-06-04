use reqwest::Url;
use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Serialize;

use crate::http::error::BurnCentralHttpError;
use crate::schemas::BurnCentralCodeMetadata;
use crate::{
    client::BurnCentralCredentials,
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

impl From<reqwest::Error> for BurnCentralHttpError {
    fn from(error: reqwest::Error) -> Self {
        match error.status() {
            Some(status) => BurnCentralHttpError::HttpError(status, error.to_string()),
            None => BurnCentralHttpError::UnknownError(error.to_string()),
        }
    }
}

trait ResponseExt {
    fn map_to_burn_central_err(self) -> Result<reqwest::blocking::Response, BurnCentralHttpError>;
}

impl ResponseExt for reqwest::blocking::Response {
    fn map_to_burn_central_err(self) -> Result<reqwest::blocking::Response, BurnCentralHttpError> {
        if self.status().is_success() {
            Ok(self)
        } else {
            Err(BurnCentralHttpError::HttpError(self.status(), self.text()?))
        }
    }
}

/// A client for making HTTP requests to the Burn Central API.
///
/// The client can be used to interact with the Burn Central server, such as creating and starting experiments, saving and loading checkpoints, and uploading logs.
#[derive(Debug, Clone)]
pub struct HttpClient {
    http_client: reqwest::blocking::Client,
    base_url: Url,
    ws_secure: bool,
    session_cookie: Option<String>,
}

impl HttpClient {
    /// Create a new HttpClient with the given base URL and API key.
    pub fn new(base_url: Url, ws_secure: bool) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            ws_secure,
            session_cookie: None,
        }
    }

    /// Check if the Burn Central server is reachable.
    #[allow(dead_code)]
    pub fn health_check(&self) -> Result<(), BurnCentralHttpError> {
        let url = self.join("health");
        self.http_client
            .get(url)
            .send()?
            .map_to_burn_central_err()?;
        Ok(())
    }

    /// Get the session cookie if it exists.
    pub fn get_session_cookie(&self) -> Option<&String> {
        self.session_cookie.as_ref()
    }

    /// Join the given path to the base URL.
    fn join(&self, path: &str) -> Url {
        self.base_url
            .join(path)
            .expect("Should be able to join url")
    }

    /// Log in to the Burn Central server with the given credentials.
    pub fn login(
        &mut self,
        credentials: &BurnCentralCredentials,
    ) -> Result<(), BurnCentralHttpError> {
        let url = self.join("login/api-key");

        let res = self
            .http_client
            .post(url)
            .form::<BurnCentralCredentials>(credentials)
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
                return Err(BurnCentralHttpError::BadSessionId);
            }
        } else {
            let error_message: String =
                format!("Cannot connect to Burn Central server({:?})", res.text()?);
            return Err(BurnCentralHttpError::HttpError(status, error_message));
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
        let mut url = self.join(&format!(
            "projects/{}/{}/experiments/{}/ws",
            owner_name, project_name, exp_num
        ));
        url.set_scheme(if self.ws_secure { "wss" } else { "ws" })
            .expect("Should be able to set ws scheme");

        url.to_string()
    }

    /// Create a new experiment for the given project.
    ///
    /// The client must be logged in before calling this method.
    pub fn create_experiment(
        &self,
        owner_name: &str,
        project_name: &str,
    ) -> Result<Experiment, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments",
            owner_name, project_name
        ));

        // Create a new experiment
        let experiment_response = self
            .http_client
            .post(url)
            .json(&serde_json::json!({}))
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_burn_central_err()?
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
    ) -> Result<(), BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let json = StartExperimentSchema {
            config: serde_json::to_value(config).unwrap(),
        };

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/start",
            owner_name, project_name, exp_num
        ));

        // Start the experiment
        self.http_client
            .put(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&json)
            .send()?
            .map_to_burn_central_err()?;

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
    ) -> Result<(), BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/end",
            owner_name, project_name, exp_num
        ));

        let end_status: EndExperimentSchema = match end_status {
            EndExperimentStatus::Success => EndExperimentSchema::Success,
            EndExperimentStatus::Fail(reason) => EndExperimentSchema::Fail(reason),
        };

        self.http_client
            .put(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&end_status)
            .send()?
            .map_to_burn_central_err()?;

        Ok(())
    }

    /// Save the checkpoint data to the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_save_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/checkpoints/{}",
            owner_name, project_name, exp_num, file_name
        ));

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_burn_central_err()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(save_url)
    }

    /// Request a URL to load the checkpoint data from the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_checkpoint_load_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/checkpoints/{}",
            owner_name, project_name, exp_num, file_name
        ));

        let load_url = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_burn_central_err()?
            .json::<URLSchema>()
            .map(|res| res.url)?;

        Ok(load_url)
    }

    /// Request a URL to save the final model to the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_final_model_save_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> Result<String, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/save_model",
            owner_name, project_name, exp_num
        ));

        let save_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_burn_central_err()?
            .json::<URLSchema>()?
            .url;

        Ok(save_url)
    }

    /// Request a URL to upload logs to the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_logs_upload_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> Result<String, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/experiments/{}/logs",
            owner_name, project_name, exp_num
        ));

        let logs_upload_url = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?
            .map_to_burn_central_err()?
            .json::<URLSchema>()?
            .url;

        Ok(logs_upload_url)
    }

    /// Generic method to upload bytes to the given URL.
    pub fn upload_bytes_to_url(
        &self,
        url: &str,
        bytes: Vec<u8>,
    ) -> Result<(), BurnCentralHttpError> {
        self.http_client
            .put(url)
            .body(bytes)
            .send()?
            .map_to_burn_central_err()?;

        Ok(())
    }

    /// Generic method to download bytes from the given URL.
    pub fn download_bytes_from_url(&self, url: &str) -> Result<Vec<u8>, BurnCentralHttpError> {
        let data = self
            .http_client
            .get(url)
            .send()?
            .map_to_burn_central_err()?
            .bytes()?
            .to_vec();

        Ok(data)
    }

    fn validate_session_cookie(&self) -> Result<(), BurnCentralHttpError> {
        if self.session_cookie.is_none() {
            return Err(BurnCentralHttpError::BadSessionId);
        }
        Ok(())
    }

    pub fn publish_project_version_urls(
        &self,
        owner_name: &str,
        project_name: &str,
        target_package_name: &str,
        code_metadata: BurnCentralCodeMetadata,
        crates_metadata: Vec<CrateVersionMetadata>,
        last_commit: &str,
    ) -> Result<CodeUploadUrlsSchema, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/code/upload",
            owner_name, project_name
        ));

        let response = self
            .http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&CodeUploadParamsSchema {
                target_package_name: target_package_name.to_string(),
                burn_central_metadata: code_metadata,
                crates: crates_metadata,
                version: last_commit.to_string(),
            })
            .send()?
            .map_to_burn_central_err()?;

        let upload_urls = response.json::<CodeUploadUrlsSchema>()?;
        Ok(upload_urls)
    }

    pub(crate) fn check_project_version_exists(
        &self,
        owner_name: &str,
        project_name: &str,
        project_version: &str,
    ) -> Result<bool, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/code/{}",
            owner_name, project_name, project_version
        ));

        let response = self
            .http_client
            .get(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .send()?;

        match response.status() {
            reqwest::StatusCode::OK => Ok(true),
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            _ => Err(BurnCentralHttpError::HttpError(
                response.status(),
                response.text()?,
            )),
        }
    }

    pub fn start_remote_job(
        &self,
        runner_group_name: &str,
        owner_name: &str,
        project_name: &str,
        project_version: &str,
        command: String,
    ) -> Result<(), BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{}/{}/jobs/queue",
            owner_name, project_name
        ));

        let body = RunnerQueueJobParamsSchema {
            runner_group_name: runner_group_name.to_string(),
            project_version: project_version.to_string(),
            command,
        };

        self.http_client
            .post(url)
            .header(COOKIE, self.session_cookie.as_ref().unwrap())
            .json(&body)
            .send()?
            .map_to_burn_central_err()?;

        Ok(())
    }
}
