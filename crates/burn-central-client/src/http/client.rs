use reqwest::Url;
use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Serialize;

use super::schemas::{
    CodeUploadParamsSchema, CodeUploadUrlsSchema, CreateExperimentResponseSchema,
    EndExperimentSchema, ProjectSchema, RunnerQueueJobParamsSchema, URLSchema, UserResponseSchema,
};
use crate::http::CreateProjectSchema;
use crate::http::error::BurnCentralHttpError;
use crate::schemas::BurnCentralCodeMetadata;
use crate::{
    client::BurnCentralCredentials,
    http::schemas::StartExperimentSchema,
    schemas::{CrateVersionMetadata, Experiment},
};

pub enum EndExperimentStatus {
    Success,
    Fail(String),
}

impl From<reqwest::Error> for BurnCentralHttpError {
    fn from(error: reqwest::Error) -> Self {
        match error.status() {
            Some(status) => BurnCentralHttpError::HttpError {
                status,
                body: error.to_string(),
            },
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
            Err(BurnCentralHttpError::HttpError {
                status: self.status(),
                body: self.text()?,
            })
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
    session_cookie: Option<String>,
}

impl HttpClient {
    /// Create a new HttpClient with the given base URL and API key.
    pub fn new(base_url: Url) -> Self {
        Self {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            session_cookie: None,
        }
    }

    pub fn get_json<R>(&self, path: impl AsRef<str>) -> Result<R, BurnCentralHttpError>
    where
        R: for<'de> serde::Deserialize<'de>,
    {
        let response = self.req(reqwest::Method::GET, path, None::<serde_json::Value>)?;
        let json = response.json::<R>()?;
        Ok(json)
    }

    pub fn post_json<T, R>(
        &self,
        path: impl AsRef<str>,
        body: Option<T>,
    ) -> Result<R, BurnCentralHttpError>
    where
        T: serde::Serialize,
        R: for<'de> serde::Deserialize<'de>,
    {
        let response = self.req(reqwest::Method::POST, path, body)?;
        let json = response.json::<R>()?;
        Ok(json)
    }

    pub fn post<T>(
        &self,
        path: impl AsRef<str>,
        body: Option<T>,
    ) -> Result<(), BurnCentralHttpError>
    where
        T: serde::Serialize,
    {
        self.req(reqwest::Method::POST, path, body).map(|_| ())
    }

    pub fn put<T>(&self, path: impl AsRef<str>, body: Option<T>) -> Result<(), BurnCentralHttpError>
    where
        T: serde::Serialize,
    {
        self.req(reqwest::Method::PUT, path, body).map(|_| ())
    }

    fn req<T: serde::Serialize>(
        &self,
        method: reqwest::Method,
        path: impl AsRef<str>,
        body: Option<T>,
    ) -> Result<reqwest::blocking::Response, BurnCentralHttpError> {
        let url = self.join(path.as_ref());
        let request_builder = self.http_client.request(method, url);

        let mut request_builder = if let Some(body) = body {
            request_builder.json(&body)
        } else {
            request_builder
        };

        if let Some(cookie) = self.session_cookie.as_ref() {
            request_builder = request_builder.header(COOKIE, cookie);
        }

        let response = request_builder.send()?.map_to_burn_central_err()?;

        Ok(response)
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
            return Err(BurnCentralHttpError::HttpError {
                status,
                body: error_message,
            });
        }

        Ok(())
    }

    pub fn get_current_user(&self) -> Result<UserResponseSchema, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join("user/me");

        self.get_json::<UserResponseSchema>(url)
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
        url.set_scheme(if self.base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .expect("Should be able to set ws scheme");

        url.to_string()
    }

    pub fn create_project(
        &self,
        owner_name: &str,
        project_name: &str,
        project_description: Option<&str>,
    ) -> Result<ProjectSchema, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join("user/projects");

        let project_data = CreateProjectSchema {
            name: project_name.to_string(),
            description: project_description.map(|desc| desc.to_string()),
        };

        self.post_json::<CreateProjectSchema, ProjectSchema>(url, Some(project_data))
    }

    pub fn get_project(
        &self,
        owner_name: &str,
        project_name: &str,
    ) -> Result<ProjectSchema, BurnCentralHttpError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{}/{}", owner_name, project_name));

        self.get_json::<ProjectSchema>(url)
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
            .post_json::<serde_json::Value, CreateExperimentResponseSchema>(
                url,
                None::<serde_json::Value>,
            )?;

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
        self.put::<StartExperimentSchema>(url, Some(json))
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

        self.put::<EndExperimentSchema>(url, Some(end_status))
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
            .post_json::<serde_json::Value, URLSchema>(url, None::<serde_json::Value>)
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

        let load_url = self.get_json::<URLSchema>(url).map(|res| res.url)?;

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
            .post_json::<serde_json::Value, URLSchema>(url, None::<serde_json::Value>)
            .map(|res| res.url)?;

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
            .post_json::<serde_json::Value, URLSchema>(url, None::<serde_json::Value>)
            .map(|res| res.url)?;
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

        self.post_json(
            url,
            Some(CodeUploadParamsSchema {
                target_package_name: target_package_name.to_string(),
                burn_central_metadata: code_metadata,
                crates: crates_metadata,
                version: last_commit.to_string(),
            }),
        )
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

        let response = self.req(reqwest::Method::GET, url, None::<serde_json::Value>)?;

        match response.status() {
            reqwest::StatusCode::OK => Ok(true),
            reqwest::StatusCode::NOT_FOUND => Ok(false),
            _ => Err(BurnCentralHttpError::HttpError {
                status: response.status(),
                body: response.text()?,
            }),
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

        self.post(url, Some(body))
    }
}
