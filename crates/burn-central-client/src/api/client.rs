use reqwest::Url;
use reqwest::header::{COOKIE, SET_COOKIE};
use serde::Serialize;

use super::schemas::{
    CodeUploadParamsSchema, CodeUploadUrlsSchema, CreateExperimentResponseSchema,
    EndExperimentSchema, ProjectSchema, RunnerQueueJobParamsSchema, URLSchema, UserResponseSchema,
};
use crate::api::error::{ApiErrorBody, ApiErrorCode, ClientError};
use crate::api::{CreateProjectSchema, GetUserOrganizationsResponseSchema};
use crate::schemas::BurnCentralCodeMetadata;
use crate::{
    api::schemas::StartExperimentSchema,
    credentials::BurnCentralCredentials,
    schemas::{CrateVersionMetadata, Experiment},
};

pub enum EndExperimentStatus {
    Success,
    Fail(String),
}

impl From<reqwest::Error> for ClientError {
    fn from(error: reqwest::Error) -> Self {
        match error.status() {
            Some(status) => ClientError::ApiError {
                status,
                body: ApiErrorBody {
                    code: ApiErrorCode::Unknown,
                    message: error.to_string(),
                },
            },
            None => ClientError::UnknownError(error.to_string()),
        }
    }
}

trait ResponseExt {
    fn map_to_burn_central_err(self) -> Result<reqwest::blocking::Response, ClientError>;
}

impl ResponseExt for reqwest::blocking::Response {
    fn map_to_burn_central_err(self) -> Result<reqwest::blocking::Response, ClientError> {
        if self.status().is_success() {
            Ok(self)
        } else {
            match self.status() {
                reqwest::StatusCode::NOT_FOUND => Err(ClientError::NotFound),
                reqwest::StatusCode::UNAUTHORIZED => Err(ClientError::Unauthorized),
                reqwest::StatusCode::FORBIDDEN => Err(ClientError::Forbidden),
                reqwest::StatusCode::INTERNAL_SERVER_ERROR => Err(ClientError::InternalServerError),
                _ => Err(ClientError::ApiError {
                    status: self.status(),
                    body: self
                        .text()
                        .map_err(|e| ClientError::UnknownError(e.to_string()))?
                        .parse::<serde_json::Value>()
                        .and_then(serde_json::from_value::<ApiErrorBody>)
                        .unwrap_or_else(|e| ApiErrorBody {
                            code: ApiErrorCode::Unknown,
                            message: e.to_string(),
                        }),
                }),
            }
        }
    }
}

/// A client for making HTTP requests to the Burn Central API.
///
/// The client can be used to interact with the Burn Central server, such as creating and starting experiments, saving and loading checkpoints, and uploading logs.
#[derive(Debug, Clone)]
pub struct Client {
    http_client: reqwest::blocking::Client,
    base_url: Url,
    session_cookie: Option<String>,
}

impl Client {
    /// Create a new HttpClient with the given base URL and API key.
    pub fn new(base_url: Url, credentials: &BurnCentralCredentials) -> Result<Self, ClientError> {
        let mut client = Self::new_without_credentials(base_url);
        let cookie = client.login(credentials)?;
        client.session_cookie = Some(cookie);
        Ok(client)
    }

    /// Create a new HttpClient without credentials.
    pub fn new_without_credentials(base_url: Url) -> Self {
        Client {
            http_client: reqwest::blocking::Client::new(),
            base_url,
            session_cookie: None,
        }
    }

    pub fn get_json<R>(&self, path: impl AsRef<str>) -> Result<R, ClientError>
    where
        R: for<'de> serde::Deserialize<'de>,
    {
        let response = self.req(reqwest::Method::GET, path, None::<serde_json::Value>)?;
        let json = response.json::<R>()?;
        Ok(json)
    }

    pub fn post_json<T, R>(&self, path: impl AsRef<str>, body: Option<T>) -> Result<R, ClientError>
    where
        T: serde::Serialize,
        R: for<'de> serde::Deserialize<'de>,
    {
        let response = self.req(reqwest::Method::POST, path, body)?;
        let json = response.json::<R>()?;
        Ok(json)
    }

    pub fn post<T>(&self, path: impl AsRef<str>, body: Option<T>) -> Result<(), ClientError>
    where
        T: serde::Serialize,
    {
        self.req(reqwest::Method::POST, path, body).map(|_| ())
    }

    pub fn put<T>(&self, path: impl AsRef<str>, body: Option<T>) -> Result<(), ClientError>
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
    ) -> Result<reqwest::blocking::Response, ClientError> {
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
    pub fn health_check(&self) -> Result<(), ClientError> {
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
    fn login(&self, credentials: &BurnCentralCredentials) -> Result<String, ClientError> {
        let url = self.join("login/api-key");

        let res = self
            .http_client
            .post(url)
            .form::<BurnCentralCredentials>(credentials)
            .send()?
            .map_to_burn_central_err()?;

        let cookie_header = res.headers().get(SET_COOKIE);
        if let Some(cookie) = cookie_header {
            let cookie_str = cookie
                .to_str()
                .expect("Session cookie should be able to convert to str");
            Ok(cookie_str.to_string())
        } else {
            Err(ClientError::BadSessionId)
        }
    }

    pub fn get_current_user(&self) -> Result<UserResponseSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join("user");

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
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/ws"
        ));
        url.set_scheme(if self.base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .expect("Should be able to set ws scheme");

        url.to_string()
    }

    pub fn create_user_project(
        &self,
        project_name: &str,
        project_description: Option<&str>,
    ) -> Result<ProjectSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join("user/projects");

        let project_data = CreateProjectSchema {
            name: project_name.to_string(),
            description: project_description.map(|desc| desc.to_string()),
        };

        self.post_json::<CreateProjectSchema, ProjectSchema>(url, Some(project_data))
    }

    pub fn create_organization_project(
        &self,
        owner_name: &str,
        project_name: &str,
        project_description: Option<&str>,
    ) -> Result<ProjectSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("organizations/{owner_name}/projects"));

        let project_data = CreateProjectSchema {
            name: project_name.to_string(),
            description: project_description.map(|desc| desc.to_string()),
        };

        self.post_json::<CreateProjectSchema, ProjectSchema>(url, Some(project_data))
    }

    pub fn get_user_organizations(
        &self,
    ) -> Result<GetUserOrganizationsResponseSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join("user/organizations");

        self.get_json(url)
    }

    pub fn get_project(
        &self,
        owner_name: &str,
        project_name: &str,
    ) -> Result<ProjectSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}"));

        self.get_json::<ProjectSchema>(url)
    }

    /// Create a new experiment for the given project.
    ///
    /// The client must be logged in before calling this method.
    pub fn create_experiment(
        &self,
        owner_name: &str,
        project_name: &str,
    ) -> Result<Experiment, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/experiments"));

        // Create a new experiment
        let experiment_response = self
            .post_json::<serde_json::Value, CreateExperimentResponseSchema>(
                url,
                Some(serde_json::json!({})),
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
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let json = StartExperimentSchema {
            config: serde_json::to_value(config)?,
        };

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/start"
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
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/end"
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
    pub fn request_artifact_save_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{file_name}"
        ));

        let save_url = self
            .post_json::<serde_json::Value, URLSchema>(url, None::<serde_json::Value>)
            .map(|res| res.url)?;

        Ok(save_url)
    }

    /// Request a URL to load the checkpoint data from the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn request_artifact_load_url(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        file_name: &str,
    ) -> Result<String, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{file_name}"
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
    ) -> Result<String, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/save_model"
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
    ) -> Result<String, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/logs"
        ));

        let logs_upload_url = self
            .post_json::<serde_json::Value, URLSchema>(url, None::<serde_json::Value>)
            .map(|res| res.url)?;
        Ok(logs_upload_url)
    }

    /// Generic method to upload bytes to the given URL.
    pub fn upload_bytes_to_url(&self, url: &str, bytes: Vec<u8>) -> Result<(), ClientError> {
        self.http_client
            .put(url)
            .body(bytes)
            .send()?
            .map_to_burn_central_err()?;

        Ok(())
    }

    /// Generic method to download bytes from the given URL.
    pub fn download_bytes_from_url(&self, url: &str) -> Result<Vec<u8>, ClientError> {
        let data = self
            .http_client
            .get(url)
            .send()?
            .map_to_burn_central_err()?
            .bytes()?
            .to_vec();

        Ok(data)
    }

    fn validate_session_cookie(&self) -> Result<(), ClientError> {
        if self.session_cookie.is_none() {
            return Err(ClientError::BadSessionId);
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
    ) -> Result<CodeUploadUrlsSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/code/upload"));

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

    pub fn check_project_version_exists(
        &self,
        owner_name: &str,
        project_name: &str,
        project_version: &str,
    ) -> Result<bool, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/code/{project_version}"
        ));

        match self.get_json::<serde_json::Value>(url) {
            Ok(_) => Ok(true),
            Err(ClientError::ApiError {
                status: reqwest::StatusCode::NOT_FOUND,
                ..
            }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn start_remote_job(
        &self,
        runner_group_name: &str,
        owner_name: &str,
        project_name: &str,
        project_version: &str,
        command: String,
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/jobs/queue"));

        let body = RunnerQueueJobParamsSchema {
            runner_group_name: runner_group_name.to_string(),
            project_version: project_version.to_string(),
            command,
        };

        self.post(url, Some(body))
    }
}
