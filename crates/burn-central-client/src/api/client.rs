use reqwest::Url;
use reqwest::header::{COOKIE, SET_COOKIE};

use super::schemas::{
    CodeUploadParamsSchema, CodeUploadUrlsSchema, ComputeProviderQueueJobParamsSchema,
    ExperimentResponse, ProjectSchema, UserResponseSchema,
};
use crate::api::error::{ApiErrorBody, ApiErrorCode, ClientError};
use crate::api::{
    AddFilesToArtifactRequest, ArtifactAddFileResponse, ArtifactCreationResponse,
    ArtifactDownloadResponse, ArtifactFileSpecRequest, ArtifactListResponse, ArtifactResponse,
    CompleteUploadRequest, CreateArtifactRequest, CreateProjectSchema,
    GetUserOrganizationsResponseSchema, ModelDownloadResponse, ModelResponse, ModelVersionResponse,
};
use crate::schemas::{BurnCentralCodeMetadata, CreatedByUser};
use crate::{
    api::schemas::CreateExperimentSchema,
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

    pub fn get_json_with_body<T, R>(
        &self,
        path: impl AsRef<str>,
        body: Option<T>,
    ) -> Result<R, ClientError>
    where
        T: serde::Serialize,
        R: for<'de> serde::Deserialize<'de>,
    {
        let response = self.req(reqwest::Method::GET, path, body)?;
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
        request_builder = request_builder.header("X-SDK-Version", env!("CARGO_PKG_VERSION"));

        let response = request_builder.send()?.map_to_burn_central_err()?;

        Ok(response)
    }

    /// Get the session cookie if it exists.
    pub fn get_session_cookie(&self) -> Option<&String> {
        self.session_cookie.as_ref()
    }

    // Todo update to support multiple versions
    fn join(&self, path: &str) -> Url {
        self.join_versioned(path, 1)
    }

    fn join_versioned(&self, path: &str, version: u8) -> Url {
        self.base_url
            .join(&format!("v{version}/"))
            .unwrap()
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
        description: Option<String>,
        code_version_digest: String,
        routine: String,
    ) -> Result<Experiment, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/experiments"));

        // Create a new experiment
        let experiment_response = self.post_json::<CreateExperimentSchema, ExperimentResponse>(
            url,
            Some(CreateExperimentSchema {
                description,
                code_version_digest,
                routine_run: routine,
            }),
        )?;

        let experiment = Experiment {
            experiment_num: experiment_response.experiment_num,
            project_name: project_name.to_string(),
            status: experiment_response.status,
            description: experiment_response.description,
            config: experiment_response.config,
            created_by: CreatedByUser {
                id: experiment_response.created_by.id,
                username: experiment_response.created_by.username,
                namespace: experiment_response.created_by.namespace,
            },
            created_at: experiment_response.created_at,
        };

        Ok(experiment)
    }

    /// Creates an artifact entry on the Burn Central server with the given files.
    ///
    /// The client must be logged in before calling this method.
    pub fn create_artifact(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        req: CreateArtifactRequest,
    ) -> Result<ArtifactCreationResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts"
        ));

        self.post_json::<CreateArtifactRequest, ArtifactCreationResponse>(url, Some(req))
    }

    /// Add files to an existing artifact.
    ///
    /// The client must be logged in before calling this method.
    pub fn add_files_to_artifact(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        artifact_id: &str,
        files: Vec<ArtifactFileSpecRequest>,
    ) -> Result<ArtifactAddFileResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{artifact_id}/files"
        ));

        self.post_json::<AddFilesToArtifactRequest, ArtifactAddFileResponse>(
            url,
            Some(AddFilesToArtifactRequest { files }),
        )
    }

    /// Complete an artifact upload.
    ///
    /// The client must be logged in before calling this method.
    ///
    /// If `file_names` is None, all files in the artifact will be marked as complete.
    /// If `file_names` is Some, only the specified files will be marked as complete.
    pub fn complete_artifact_upload(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        artifact_id: &str,
        file_names: Option<Vec<String>>,
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{artifact_id}/complete"
        ));

        let body = Some(CompleteUploadRequest { file_names });
        self.post(url, body)
    }

    /// List artifacts for the given experiment.
    ///
    /// The client must be logged in before calling this method.
    pub fn list_artifacts(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
    ) -> Result<ArtifactListResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts"
        ));

        self.get_json::<ArtifactListResponse>(url)
    }

    /// Query artifacts by name for the given experiment.
    ///
    /// The client must be logged in before calling this method.
    pub fn list_artifacts_by_name(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        name: &str,
    ) -> Result<ArtifactListResponse, ClientError> {
        self.validate_session_cookie()?;

        let mut url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts"
        ));
        url.query_pairs_mut().append_pair("name", name);

        self.get_json::<ArtifactListResponse>(url)
    }

    /// Get details about a specific artifact by its ID.
    ///
    /// The client must be logged in before calling this method.
    pub fn get_artifact(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        artifact_id: &str,
    ) -> Result<ArtifactResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{artifact_id}"
        ));

        self.get_json::<ArtifactResponse>(url)
    }

    /// Request presigned URLs to download an artifact's files from the Burn Central server.
    ///
    /// The client must be logged in before calling this method.
    pub fn presign_artifact_download(
        &self,
        owner_name: &str,
        project_name: &str,
        exp_num: i32,
        artifact_id: &str,
    ) -> Result<ArtifactDownloadResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/experiments/{exp_num}/artifacts/{artifact_id}/download"
        ));

        self.get_json::<ArtifactDownloadResponse>(url)
    }

    /// Get details about a specific model.
    ///
    /// The client must be logged in before calling this method.
    pub fn get_model(
        &self,
        namespace: &str,
        project_name: &str,
        model_name: &str,
    ) -> Result<ModelResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{namespace}/{project_name}/models/{model_name}"
        ));

        self.get_json::<ModelResponse>(url)
    }

    /// Get details about a specific model version.
    ///
    /// The client must be logged in before calling this method.
    pub fn get_model_version(
        &self,
        namespace: &str,
        project_name: &str,
        model_name: &str,
        version: u32,
    ) -> Result<ModelVersionResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{namespace}/{project_name}/models/{model_name}/versions/{version}"
        ));

        self.get_json::<ModelVersionResponse>(url)
    }

    /// Generate presigned URLs for downloading model version files.
    ///
    /// The client must be logged in before calling this method.
    pub fn presign_model_download(
        &self,
        namespace: &str,
        project_name: &str,
        model_name: &str,
        version: u32,
    ) -> Result<ModelDownloadResponse, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{namespace}/{project_name}/models/{model_name}/versions/{version}/download"
        ));

        self.get_json::<ModelDownloadResponse>(url)
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
        digest: &str,
    ) -> Result<CodeUploadUrlsSchema, ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/code/upload"));

        self.post_json(
            url,
            Some(CodeUploadParamsSchema {
                target_package_name: target_package_name.to_string(),
                burn_central_metadata: code_metadata,
                crates: crates_metadata,
                digest: digest.to_string(),
            }),
        )
    }

    pub fn complete_project_version_upload(
        &self,
        owner_name: &str,
        project_name: &str,
        code_version_id: &str,
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!(
            "projects/{owner_name}/{project_name}/code/{code_version_id}/complete"
        ));

        self.post(url, None::<()>)
    }

    pub fn start_remote_job(
        &self,
        compute_provider_group_name: &str,
        owner_name: &str,
        project_name: &str,
        digest: &str,
        command: &str,
    ) -> Result<(), ClientError> {
        self.validate_session_cookie()?;

        let url = self.join(&format!("projects/{owner_name}/{project_name}/jobs/queue"));

        let body = ComputeProviderQueueJobParamsSchema {
            compute_provider_group_name: compute_provider_group_name.to_string(),
            digest: digest.to_string(),
            command: command.to_string(),
        };

        self.post(url, Some(body))
    }
}
