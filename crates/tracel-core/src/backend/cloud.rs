use std::path::Path;

use serde::Deserialize;
use tracel_artifact::ReqwestTransferClient;
use tracel_client::{Client, ClientError, Env, TracelCredentials};

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
    pub(crate) model_cache: crate::model_registry::ModelCache,
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
            .join(&namespace)
            .join(&project)
            .join("models");

        Ok(Self {
            client,
            namespace,
            project,
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: crate::model_registry::ModelCache::new(cache_root),
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
