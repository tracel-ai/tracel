//! Discovering how to reach the console: environment, credentials, and which project.

use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use tracel_client::{
    ClientError,
    console::{
        AppSession, Env, FileSessionStore, SessionStore, TracelCredentials, auth::DeviceAuthClient,
    },
};

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";

/// The client `tracel login` signs in as; renewing its session must name it too.
const TRACEL_CLI_CLIENT_ID: &str = "tracel-cli";

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("No credentials found: set {TRACEL_API_KEY} or run `tracel login`")]
    NoCredentials,
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Deserialize, Default)]
struct TracelTomlConfig {
    #[serde(alias = "owner")]
    namespace: Option<String>,
    #[serde(alias = "name")]
    project: Option<String>,
}

/// `TRACEL_API_KEY` first, then the app session `tracel login` stored for this
/// environment's server, which the client renews as the run goes.
pub fn discover_credentials() -> Result<TracelCredentials, CloudError> {
    if let Ok(credentials) = TracelCredentials::from_env() {
        return Ok(credentials);
    }

    let env = discover_env();
    let store = FileSessionStore::for_server(&env.get_url()).map_err(ClientError::from)?;
    if store.load().map_err(ClientError::from)?.is_none() {
        return Err(CloudError::NoCredentials);
    }

    let device_auth = DeviceAuthClient::new(env, TRACEL_CLI_CLIENT_ID);
    Ok(TracelCredentials::app_session(AppSession::new(
        Arc::new(store),
        device_auth,
    )))
}

pub fn discover_namespace_project() -> Result<(String, String), CloudError> {
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

fn discover_env() -> Env {
    let Ok(value) = std::env::var(TRACEL_ENV) else {
        return Env::Production;
    };

    match value.as_str() {
        "Development" => Env::Development,
        other => other
            .strip_prefix("Staging(")
            .and_then(|rest| rest.strip_suffix(')'))
            .and_then(|number| number.parse().ok())
            .map(Env::Staging)
            .unwrap_or(Env::Production),
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
