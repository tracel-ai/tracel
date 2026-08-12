use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use tracel_artifact::ReqwestTransferClient;
use tracel_client::{Client, ClientError, Env, TracelCredentials};
use tracel_models::{
    Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile, VersionFileSource,
    VersionId, VersionManifest,
};

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    /// No API key was available from the environment or Tracel CLI configuration.
    #[error("No API key found: set {TRACEL_API_KEY} or run `tracel login`")]
    NoCredentials,
    /// The console rejected the discovered API key.
    #[error("API key is invalid or has expired: run `tracel login` to log in again")]
    InvalidCredentials,
    /// No namespace was available from the environment or `tracel.toml`.
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    /// No project was available from the environment or `tracel.toml`.
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    /// `TRACEL_ENV` did not identify a supported console environment.
    #[error("Invalid environment variable {env_var}: {message}")]
    InvalidEnv {
        /// Name of the invalid environment variable.
        env_var: String,
        /// Expected-value guidance.
        message: String,
    },
    /// No platform cache directory could be resolved for model downloads.
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
    /// The console client failed while authenticating or communicating.
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Clone)]
pub struct CloudBackend {
    pub(crate) client: Client,
    pub(crate) namespace: String,
    pub(crate) project: String,
    pub(crate) file_transfer_client: ReqwestTransferClient,
    pub(crate) model_cache: crate::model_cache::ModelCache,
    model_version_routes: crate::model_routes::ModelVersionRoutes,
}

#[derive(Deserialize)]
struct CliCredentials {
    api_key: String,
}

#[derive(Deserialize, Default)]
struct TracelTomlConfig {
    #[serde(alias = "owner")]
    namespace: Option<String>,
    #[serde(alias = "name")]
    project: Option<String>,
}

impl CloudBackend {
    /// Binds the backend to an authenticated client and explicit project coordinates.
    pub fn new(client: Client, namespace: String, project: String) -> Result<Self, CloudError> {
        let cache_root = crate::resolve_cache_dir()
            .ok_or(CloudError::NoCacheDir)?
            .join("cloud")
            .join(crate::model_cache::opaque_cache_key(
                client.base_url().as_str(),
            ))
            .join(crate::model_cache::opaque_cache_key(&namespace))
            .join(crate::model_cache::opaque_cache_key(&project))
            .join("models");

        Ok(Self {
            client,
            namespace,
            project,
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: crate::model_cache::ModelCache::new(cache_root),
            model_version_routes: crate::model_routes::ModelVersionRoutes::default(),
        })
    }

    /// Discovers credentials and project coordinates using the Tracel CLI conventions.
    pub fn discover() -> Result<CloudBackend, CloudError> {
        let env = discover_env()?;
        let credentials = discover_credentials(&env)?;
        let (namespace, project) = discover_namespace_project()?;

        let client = Client::new(env, &credentials).map_err(|err| {
            if err.is_login_error() {
                CloudError::InvalidCredentials
            } else {
                CloudError::Client(err)
            }
        })?;
        CloudBackend::new(client, namespace, project)
    }

    /// Returns the authenticated client used by this backend.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Returns the namespace that owns this backend's project.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the project name used by this backend.
    pub fn project(&self) -> &str {
        &self.project
    }

    /// Returns model operations scoped to this backend's console project.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(self.clone()))
    }

    fn resolve_model_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        if let Some(version) = self.model_version_routes.get(model, id)? {
            return Ok(version);
        }

        let response = self
            .client
            .list_model_versions(&self.namespace, &self.project, model)
            .map_err(|error| model_request_error(error, model))?;

        for version in response.items {
            self.model_version_routes.remember(
                model,
                VersionId::new(version.id),
                version.version,
            )?;
        }
        self.model_version_routes
            .get(model, id)?
            .ok_or_else(|| ModelsError::VersionNotFound {
                model: model.to_string(),
                id: id.clone(),
            })
    }

    fn version_file_source(
        &self,
        model: &str,
        id: &VersionId,
        response: tracel_client::response::PresignedModelFileUrlResponse,
    ) -> Box<dyn VersionFileSource> {
        let file = VersionFile {
            rel_path: response.rel_path,
            size_bytes: response.size_bytes,
            checksum: response.checksum,
        };
        Box::new(self.model_cache.file_source(
            model,
            id,
            file,
            response.url,
            self.file_transfer_client.clone(),
        ))
    }
}

impl ModelOps for CloudBackend {
    fn list_models(&self) -> Result<Page<Model>, ModelsError> {
        self.client
            .list_models(&self.namespace, &self.project)
            .map(|response| Page {
                items: response.items.into_iter().map(map_model).collect(),
                total: response.total,
            })
            .map_err(scope_request_error)
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.client
            .get_model(&self.namespace, &self.project, name)
            .map(map_model)
            .map_err(|error| model_request_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        let response = self
            .client
            .list_model_versions(&self.namespace, &self.project, model)
            .map_err(|error| model_request_error(error, model))?;
        for version in &response.items {
            self.model_version_routes.remember(
                model,
                VersionId::new(&version.id),
                version.version,
            )?;
        }
        let items = response
            .items
            .into_iter()
            .map(map_version)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Page {
            items,
            total: response.total,
        })
    }

    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        let version = self.resolve_model_version(model, id)?;
        self.client
            .get_model_version(&self.namespace, &self.project, model, version)
            .map_err(|error| version_request_error(error, model, id))
            .and_then(map_version)
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let version = self.resolve_model_version(model, id)?;
        self.client
            .presign_model_download(&self.namespace, &self.project, model, version)
            .map_err(|error| version_request_error(error, model, id))
            .map(|response| {
                response
                    .files
                    .into_iter()
                    .map(|file| self.version_file_source(model, id, file))
                    .collect()
            })
    }
}

fn map_model(response: tracel_client::response::ModelResponse) -> Model {
    Model {
        id: response.id,
        name: response.name,
        description: response.description,
        published_by: Some(response.created_by.username),
        created_at: response.created_at,
        version_count: response.version_count,
        latest_version: response.latest_version,
    }
}

fn map_version(
    response: tracel_client::response::ModelVersionResponse,
) -> Result<ModelVersion, ModelsError> {
    let manifest = serde_json::from_value::<VersionManifest>(response.manifest)
        .map_err(|error| ModelsError::InvalidResponse(error.to_string()))?;

    Ok(ModelVersion {
        id: VersionId::new(response.id),
        version: response.version,
        size_bytes: response.size,
        checksum: response.checksum,
        published_by: Some(response.created_by.username),
        created_at: response.created_at,
        manifest,
        metadata: response.metadata,
    })
}

fn model_request_error(error: ClientError, name: &str) -> ModelsError {
    if client_error_is_not_visible(&error) {
        ModelsError::ModelNotFound {
            name: name.to_string(),
        }
    } else {
        map_client_error(error)
    }
}

fn scope_request_error(error: ClientError) -> ModelsError {
    if client_error_is_not_visible(&error) {
        ModelsError::ScopeNotFound
    } else {
        map_client_error(error)
    }
}

fn version_request_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if client_error_is_not_visible(&error) {
        ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        }
    } else {
        map_client_error(error)
    }
}

fn map_client_error(error: ClientError) -> ModelsError {
    match error {
        ClientError::Unauthorized => ModelsError::SessionExpired,
        ClientError::ApiError { ref status, .. } if status.as_u16() == 401 => {
            ModelsError::SessionExpired
        }
        ClientError::BadSessionId => {
            ModelsError::InvalidResponse("login response omitted the session cookie".to_string())
        }
        ClientError::Serialization(error) => ModelsError::InvalidResponse(error.to_string()),
        error => ModelsError::Transport(error.to_string()),
    }
}

fn client_error_is_not_visible(error: &ClientError) -> bool {
    error.is_not_found()
        || matches!(
            error,
            ClientError::ApiError { status, .. }
                if status.as_u16() == 403 || status.as_u16() == 404
        )
}

fn discover_credentials(env: &Env) -> Result<TracelCredentials, CloudError> {
    if let Ok(creds) = TracelCredentials::from_env() {
        return Ok(creds);
    }

    let config_dir = crate::resolve_config_dir().ok_or(CloudError::NoCredentials)?;

    let filename = match env {
        Env::Production => "credentials.json".to_string(),
        Env::Staging(v) => format!("credentials-staging{v}.json"),
        Env::Development => "credentials-dev.json".to_string(),
    };

    let path = config_dir.join(&filename);
    if path.exists() {
        let contents = std::fs::read_to_string(path).map_err(|_| CloudError::NoCredentials)?;
        let creds: CliCredentials =
            serde_json::from_str(&contents).map_err(|_| CloudError::NoCredentials)?;
        return Ok(TracelCredentials::new(creds.api_key));
    }

    Err(CloudError::NoCredentials)
}

fn discover_namespace_project() -> Result<(String, String), CloudError> {
    let namespace_env = std::env::var(TRACEL_NAMESPACE).ok();
    let project_env = std::env::var(TRACEL_PROJECT).ok();

    if let (Some(ns), Some(proj)) = (&namespace_env, &project_env) {
        return Ok((ns.clone(), proj.clone()));
    }

    let toml_config = read_tracel_toml();

    let namespace = namespace_env
        .or(toml_config.namespace)
        .ok_or(CloudError::NoNamespace)?;

    let project = project_env
        .or(toml_config.project)
        .ok_or(CloudError::NoProject)?;

    Ok((namespace, project))
}

fn discover_env() -> Result<Env, CloudError> {
    let invalid_env = || CloudError::InvalidEnv {
        env_var: TRACEL_ENV.to_string(),
        message: "expected value to be one of: 'Production', 'Development', or 'Staging(N)'"
            .to_string(),
    };

    match std::env::var(TRACEL_ENV) {
        Ok(val) => match val.as_str() {
            "Production" => Ok(Env::Production),
            "Development" => Ok(Env::Development),
            other => other
                .strip_prefix("Staging(")
                .and_then(|rest| rest.strip_suffix(')'))
                .and_then(|n| n.parse::<u8>().ok())
                .map(Env::Staging)
                .ok_or_else(invalid_env),
        },
        Err(_) => Ok(Env::Production),
    }
}

fn read_tracel_toml() -> TracelTomlConfig {
    let path = Path::new("tracel.toml");
    if !path.exists() {
        return TracelTomlConfig::default();
    }
    let Ok(contents) = std::fs::read_to_string(path) else {
        return TracelTomlConfig::default();
    };
    toml::from_str(&contents).unwrap_or_default()
}
