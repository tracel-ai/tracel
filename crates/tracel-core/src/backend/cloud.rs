use std::{path::Path, sync::Arc, task::Context};

use serde::Deserialize;
use tracel_artifact::ReqwestTransferClient;
use tracel_client::{Client, ClientError, Env, TracelCredentials};

use crate::{
    context::{IntoProviders, Providers},
    inference::CloudInferenceProvider,
    model_registry::ModelCache,
};

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("No API key found: set {TRACEL_API_KEY} or run `tracel login`")]
    NoCredentials,
    #[error("API key is invalid or has expired: run `tracel login` to log in again")]
    InvalidCredentials,
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    #[error("Invalid environment variable {env_var}: {message}")]
    InvalidEnv { env_var: String, message: String },
    #[error("could not read tracel.toml: {0}")]
    ReadTracelToml(#[source] std::io::Error),
    #[error("could not write tracel.toml: {0}")]
    InvalidTracelToml(#[from] toml::de::Error),
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
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

pub enum AuthMethod {
    Env,
    ApiKey(String),
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
    fn new(authentication: AuthMethod) -> Result<Self, CloudError> {
        let client = match authentication {
            AuthMethod::Env => authenticate()?,
            AuthMethod::ApiKey(api_key) => {
                Client::new(Env::Production, &TracelCredentials::new(api_key))?
            }
        };
        let (namespace, project) = discover_namespace_project()?;

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
            model_cache: ModelCache::new(cache_root),
        })
    }

    pub fn create_project(
        self,
        owner: &str,
        name: &str,
        description: &str,
    ) -> Result<(), CloudError> {
        self.client
            .create_organization_project(owner, name, Some(description))?;
        Ok(())
    }
}

impl IntoProviders for CloudBackend {
    fn into_providers(self) -> Result<crate::context::Providers, crate::ContextError> {
        let backend = Arc::new(self.clone());
        let inference = Arc::new(CloudInferenceProvider::new(
            self.client.clone(),
            self.namespace.clone(),
            self.project.clone(),
        ));
        Ok(Providers {
            experiment: backend.clone(),
            inference,
            model_registry: Some(backend),
            dataset: None,
        })
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

    let toml_config = read_tracel_toml()?;

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

fn authenticate() -> Result<Client, CloudError> {
    let env = discover_env()?;
    let credentials = discover_credentials(&env)?;
    let client = Client::new(env, &credentials).map_err(|err| {
        if err.is_login_error() {
            CloudError::InvalidCredentials
        } else {
            CloudError::Client(err)
        }
    })?;
    Ok(client)
}

fn read_tracel_toml() -> Result<TracelTomlConfig, CloudError> {
    let path = Path::new("tracel.toml");
    if !path.exists() {
        return Ok(TracelTomlConfig::default());
    }
    let contents = std::fs::read_to_string(path).map_err(CloudError::ReadTracelToml)?;
    Ok(toml::from_str(&contents)?)
}
