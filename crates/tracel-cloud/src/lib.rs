use std::path::Path;

use burn_central_client::BurnCentralCredentials;
use burn_central_client::Client;
use burn_central_client::ClientError;
use burn_central_client::Env;
use module::experiment::Experiment;
use module::experiment::RunProvider;
use serde::Deserialize;
use tracel_experiment::ExperimentRun;

mod module;

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("No API key found — set BURN_CENTRAL_API_KEY or run `burn login`")]
    NoCredentials,
    #[error("No namespace found — set TRACEL_NAMESPACE or add namespace to tracel.toml")]
    NoNamespace,
    #[error("No project found — set TRACEL_PROJECT or add project to tracel.toml")]
    NoProject,
    #[error(transparent)]
    Client(#[from] ClientError),
}

#[derive(Debug, Clone)]
pub struct Context {
    pub backend: Backend,
    pub namespace: String,
    pub project: String,
}

#[derive(Debug, Clone)]
pub enum Backend {
    Cloud(Client),
    Local,
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

impl Context {
    fn new(backend: Backend, namespace: String, project: String) -> Self {
        Self {
            backend,
            namespace,
            project,
        }
    }

    pub fn cloud(env: Env) -> Result<Self, DiscoverError> {
        Self::discover(env)
    }

    pub fn local() -> Result<Self, DiscoverError> {
        eprintln!(
            "[tracel] running in local mode (no credentials or connection to Burn Central will be used)"
        );
        let (namespace, project) = discover_namespace_project()?;

        Ok(Self {
            backend: Backend::Local,
            namespace,
            project,
        })
    }

    pub fn experiment(&self) -> Experiment<Context> {
        Experiment::new(self.clone())
    }

    /// Discover credentials, namespace, and project automatically.
    ///
    /// First checks env var and if no credentials, then check CLI or tracel.toml
    ///
    /// TODO : remove env option to user
    fn discover(env: Env) -> Result<Self, DiscoverError> {
        let credentials = discover_credentials(&env)?;
        let (namespace, project) = discover_namespace_project()?;

        let client = Client::new(env, &credentials).map_err(DiscoverError::Client)?;
        let backend = Backend::Cloud(client);

        Ok(Self::new(backend, namespace, project))
    }
}

impl RunProvider for Context {
    fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, String> {
        let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

        let digest = "46523358ec1646354ddab1cd8b93f2b920b44b24a26ea86c129d666d6bae2a5f".to_string();

        match self.backend.clone() {
            Backend::Cloud(client) => {
                ExperimentRun::cloud(client, &self.namespace, &self.project, digest, routine)
                    .map_err(|e| {
                        use std::error::Error;
                        let mut msg =
                            format!("An error occured while creating the experiment: {e}");
                        let mut src = e.source();
                        while let Some(s) = src {
                            msg.push_str(&format!("caused by: {s}"));
                            src = s.source();
                        }
                        msg
                    })
            }
            Backend::Local => ExperimentRun::local("./runs").map_err(|e| e.to_string()),
        }
    }
}

fn discover_credentials(env: &Env) -> Result<BurnCentralCredentials, DiscoverError> {
    if let Ok(creds) = BurnCentralCredentials::from_env() {
        eprintln!("[tracel] credentials found via environment variable");
        return Ok(creds);
    }

    let proj_dirs = directories::ProjectDirs::from("com", "tracel", "burncentral")
        .ok_or(DiscoverError::NoCredentials)?;

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
        let contents = std::fs::read_to_string(path).map_err(|_| DiscoverError::NoCredentials)?;
        let creds: CliCredentials =
            serde_json::from_str(&contents).map_err(|_| DiscoverError::NoCredentials)?;
        return Ok(BurnCentralCredentials::new(creds.api_key));
    }

    Err(DiscoverError::NoCredentials)
}

fn discover_namespace_project() -> Result<(String, String), DiscoverError> {
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
        .or_else(|| toml_config.namespace)
        .ok_or(DiscoverError::NoNamespace)?;

    let project = project_env
        .or_else(|| toml_config.project)
        .ok_or(DiscoverError::NoProject)?;

    eprintln!("[tracel] namespace found via {ns_source}: {namespace}");
    eprintln!("[tracel] project found via {proj_source}: {project}");

    Ok((namespace, project))
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
