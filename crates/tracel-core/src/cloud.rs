//! Discovering how to reach the console: environment, credentials, and which project.

use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};
use serde::Deserialize;
use tracel_client::{
    ClientError,
    console::{Env, TracelCredentials},
};

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("No API key found: set {TRACEL_API_KEY} or run `tracel login`")]
    NoCredentials,
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    #[error(transparent)]
    Client(#[from] ClientError),
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

pub fn discover_credentials() -> Result<TracelCredentials, CloudError> {
    if let Ok(creds) = TracelCredentials::from_env() {
        return Ok(creds);
    }

    let env = discover_env();

    let config_dir = resolve_config_dir().ok_or(CloudError::NoCredentials)?;

    let filename = match &env {
        Env::Production => "credentials.json".to_string(),
        Env::Staging(v) => format!("credentials-staging{v}.json"),
        Env::Development => "credentials-dev.json".to_string(),
    };

    let path = config_dir.join(&filename);
    if path.exists() {
        let contents = std::fs::read_to_string(path).map_err(|_| CloudError::NoCredentials)?;
        let creds: CliCredentials =
            serde_json::from_str(&contents).map_err(|_| CloudError::NoCredentials)?;
        return Ok(TracelCredentials::api_key(creds.api_key));
    }

    Err(CloudError::NoCredentials)
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

fn resolve_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "tracel")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .or_else(|| BaseDirs::new().map(|dirs| dirs.config_dir().join("tracel")))
}
