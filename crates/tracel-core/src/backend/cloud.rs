//! Experiment backend running against the Tracel cloud.
//!
//! Two independent flows share this module:
//!
//! - **Running experiments**: [`AuthMethod`] plus [`CloudBackend::new`] (or
//!   [`authenticate`] + [`CloudBackend::from_session`]) builds a backend for
//!   [`Context::new`](crate::Context::new), scoped to an existing namespace/project.
//! - **Creating a project**: [`authenticate`] returns a [`CloudSession`], whose
//!   [`CloudSession::create_project`] creates a new cloud project. The same session can then be
//!   turned into a [`CloudBackend`] with [`CloudBackend::from_session`], scoped to the project
//!   that was just created — see that method's docs for why this needs a separate constructor
//!   from [`CloudBackend::new`].

use std::{path::Path, sync::Arc};

use serde::Deserialize;
use tracel_artifact::ReqwestTransferClient;
use tracel_client::request::Visibility;
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

/// Errors from cloud authentication, project creation, or [`CloudBackend`] construction.
#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    /// [`AuthMethod::Env`] found no credentials to authenticate with.
    #[error("No API key found: set {TRACEL_API_KEY} or run `tracel login`")]
    NoCredentials,
    /// The provided or discovered API key was rejected by the cloud.
    #[error("API key is invalid or has expired: run `tracel login` to log in again")]
    InvalidCredentials,
    /// [`CloudBackend::new`] could not resolve a namespace from the environment or `tracel.toml`.
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    /// [`CloudBackend::new`] could not resolve a project from the environment or `tracel.toml`.
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    /// `TRACEL_ENV` was set to a value other than `Production`, `Development`, or `Staging(N)`.
    #[error("Invalid environment variable {env_var}: {message}")]
    InvalidEnv {
        /// The environment variable that failed to parse.
        env_var: String,
        /// What value was expected.
        message: String,
    },
    /// `tracel.toml` exists but could not be read from disk.
    #[error("could not read tracel.toml: {0}")]
    ReadTracelToml(#[source] std::io::Error),
    /// `tracel.toml` exists but is not valid TOML.
    #[error("could not write tracel.toml: {0}")]
    InvalidTracelToml(#[from] toml::de::Error),
    /// No local cache directory could be resolved for downloaded models.
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
    /// The underlying cloud client failed to communicate with the Tracel API.
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// Experiment backend that ships telemetry to a Tracel cloud project.
///
/// Build one with [`CloudBackend::new`] (resolves namespace/project from the environment or
/// `tracel.toml`) or [`CloudBackend::from_session`] (scoped to a project you already have a
/// [`CloudSession`] for), then pass it to [`Context::new`](crate::Context::new).
#[derive(Clone)]
pub struct CloudBackend {
    pub(crate) client: Client,
    pub(crate) namespace: String,
    pub(crate) project: String,
    pub(crate) file_transfer_client: ReqwestTransferClient,
    pub(crate) model_cache: crate::model_registry::ModelCache,
}

/// How to authenticate against the Tracel cloud, passed to [`authenticate`] or
/// [`CloudBackend::new`].
pub enum AuthMethod {
    /// Discover credentials from the environment (`TRACEL_API_KEY`) or the local `tracel login`
    /// credentials file.
    Env,
    /// Authenticate with this explicit API key, against the production environment.
    ApiKey(String),
}

/// An authenticated cloud session, returned by [`authenticate`].
///
/// Use [`CloudSession::create_project`] to create a new project, and
/// [`CloudBackend::from_session`] to turn the session into a backend scoped to a project.
pub struct CloudSession {
    client: Client,
}

impl CloudSession {
    /// Creates a new project under `owner` in the Tracel cloud.
    ///
    /// Always created private for now; there's no way to request public visibility yet.
    pub fn create_project(
        &self,
        owner: &str,
        name: &str,
        description: &str,
    ) -> Result<(), CloudError> {
        self.client.create_organization_project(
            owner,
            name,
            Some(description),
            Visibility::Private,
        )?;
        Ok(())
    }

    pub(crate) fn into_client(self) -> Client {
        self.client
    }
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
    /// Builds a `CloudBackend` scoped to `namespace`/`project`, from an already-authenticated
    /// [`CloudSession`].
    ///
    /// Use this after [`CloudSession::create_project`], to run experiments in the project that
    /// was just created without relying on `TRACEL_NAMESPACE`/`TRACEL_PROJECT` or `tracel.toml`
    /// (nothing writes those automatically, and they may point elsewhere). For an existing
    /// project resolved from the environment, use [`CloudBackend::new`] instead.
    pub fn from_session(
        session: CloudSession,
        namespace: String,
        project: String,
    ) -> Result<Self, CloudError> {
        let client = session.into_client();

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

    /// Authenticates with `authentication`, then builds a `CloudBackend` for the namespace and
    /// project resolved from `TRACEL_NAMESPACE`/`TRACEL_PROJECT` or `tracel.toml`.
    pub fn new(authentication: AuthMethod) -> Result<Self, CloudError> {
        let session = authenticate(authentication)?;
        let (namespace, project) = discover_namespace_project()?;
        Self::from_session(session, namespace, project)
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

/// Authenticates against the Tracel cloud, returning a [`CloudSession`] that can create projects
/// or be turned into a [`CloudBackend`] with [`CloudBackend::from_session`].
pub fn authenticate(method: AuthMethod) -> Result<CloudSession, CloudError> {
    let (env, credentials) = match method {
        AuthMethod::Env => {
            let env = discover_env()?;
            let credentials = discover_credentials(&env)?;
            (env, credentials)
        }
        AuthMethod::ApiKey(api_key) => (Env::Production, TracelCredentials::new(api_key)),
    };

    let client = Client::new(env, &credentials).map_err(|err| {
        if err.is_login_error() {
            CloudError::InvalidCredentials
        } else {
            CloudError::Client(err)
        }
    })?;

    Ok(CloudSession { client })
}

fn read_tracel_toml() -> Result<TracelTomlConfig, CloudError> {
    let path = Path::new("tracel.toml");
    if !path.exists() {
        return Ok(TracelTomlConfig::default());
    }
    let contents = std::fs::read_to_string(path).map_err(CloudError::ReadTracelToml)?;
    Ok(toml::from_str(&contents)?)
}
