use std::path::Path;

use burn_central_client::{BurnCentralCredentials, Client, ClientError, Env};
use serde::Deserialize;

use crate::context::{Backend, Context};

const TRACEL_ENV: &str = "TRACEL_ENV";
const TRACEL_PROJECT: &str = "TRACEL_PROJECT";
const TRACEL_NAMESPACE: &str = "TRACEL_NAMESPACE";
const TRACEL_API_KEY: &str = "TRACEL_API_KEY";

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("No API key found: set {TRACEL_API_KEY} or run `burn login`")]
    NoCredentials,
    #[error("No namespace found: set {TRACEL_NAMESPACE} or add namespace to tracel.toml")]
    NoNamespace,
    #[error("No project found: set {TRACEL_PROJECT} or add project to tracel.toml")]
    NoProject,
    #[error("Invalid environment variable {env_var}: {message}")]
    InvalidEnv { env_var: String, message: String },
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Debug, Clone)]
pub struct CloudBackend {
    pub(crate) client: Client,
    pub(crate) namespace: String,
    pub(crate) project: String,
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
    fn new(client: Client, namespace: String, project: String) -> Self {
        Self {
            client,
            namespace,
            project,
        }
    }

    pub fn create_context() -> Result<Context, CloudError> {
        let env = discover_env()?;
        let credentials = discover_credentials(&env)?;
        let (namespace, project) = discover_namespace_project()?;

        let client = Client::new(env, &credentials).map_err(CloudError::Client)?;
        let cloud_backend = CloudBackend::new(client, namespace, project);

        let backend = Backend::Cloud(cloud_backend);

        Ok(Context::new(backend))
    }
}

fn discover_credentials(env: &Env) -> Result<BurnCentralCredentials, CloudError> {
    if let Ok(creds) = BurnCentralCredentials::from_env() {
        eprintln!("[tracel] credentials found via environment variable");
        return Ok(creds);
    }

    let proj_dirs = directories::ProjectDirs::from("com", "tracel", "burncentral")
        .ok_or(CloudError::NoCredentials)?;

    let filename = match env {
        Env::Production => "credentials.json".to_string(),
        Env::Staging(v) => format!("credentials-staging{v}.json"),
        Env::Development => "credentials-dev.json".to_string(),
    };

    let path = proj_dirs.config_dir().join(&filename);
    if path.exists() {
        eprintln!(
            "[tracel] credentials found in CLI config file: {}",
            path.display()
        );
        let contents = std::fs::read_to_string(path).map_err(|_| CloudError::NoCredentials)?;
        let creds: CliCredentials =
            serde_json::from_str(&contents).map_err(|_| CloudError::NoCredentials)?;
        return Ok(BurnCentralCredentials::new(creds.api_key));
    }

    Err(CloudError::NoCredentials)
}

fn discover_namespace_project() -> Result<(String, String), CloudError> {
    let namespace_env = std::env::var("TRACEL_NAMESPACE").ok();
    let project_env = std::env::var("TRACEL_PROJECT").ok();

    if let (Some(ns), Some(proj)) = (&namespace_env, &project_env) {
        eprintln!("[tracel] namespace and project found via environment variables: {ns}/{proj}");
        return Ok((ns.clone(), proj.clone()));
    }

    let toml_config = read_tracel_toml();

    let ns_source = if namespace_env.is_some() {
        "env"
    } else {
        "tracel.toml"
    };
    let proj_source = if project_env.is_some() {
        "env"
    } else {
        "tracel.toml"
    };

    let namespace = namespace_env
        .or(toml_config.namespace)
        .ok_or(CloudError::NoNamespace)?;

    let project = project_env
        .or(toml_config.project)
        .ok_or(CloudError::NoProject)?;

    eprintln!("[tracel] namespace found via {ns_source}: {namespace}");
    eprintln!("[tracel] project found via {proj_source}: {project}");

    Ok((namespace, project))
}

fn discover_env() -> Result<Env, CloudError> {
    match std::env::var(TRACEL_ENV) {
        Ok(val) => serde_json::from_str(&val).map_err(|_| CloudError::InvalidEnv {
            env_var: TRACEL_ENV.to_string(),
            message: "expected value to be one of: 'Production', 'Development', or 'Staging(N)'"
                .to_string(),
        }),
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
