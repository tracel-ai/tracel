use std::error::Error;
use std::path::Path;

use burn_central_client::BurnCentralCredentials;
use burn_central_client::Client;
use burn_central_client::ClientError;
use burn_central_client::Env;
use serde::Deserialize;
use tracel_experiment::ExperimentJob;
use tracel_experiment::ExperimentRun;
use tracel_experiment::ExperimentRunHandleExt;

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
pub struct CloudContext {
    pub client: Client,
    pub namespace: String,
    pub project: String,
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

impl CloudContext {
    fn new(
        credentials: BurnCentralCredentials,
        namespace: String,
        project: String,
        env: Env,
    ) -> Result<Self, ClientError> {
        let client = Client::new(env, &credentials)?;
        Ok(Self {
            client,
            namespace,
            project,
        })
    }

    /// Discover credentials, namespace, and project automatically.
    ///
    /// First checks env var and if no credentials, then check CLI or tracel.toml
    pub fn discover(env: Env) -> Result<Self, DiscoverError> {
        let credentials = discover_credentials(&env)?;
        let (namespace, project) = discover_namespace_project()?;
        Self::new(credentials, namespace, project, env).map_err(DiscoverError::Client)
    }

    pub fn experiment<T, F>(&self, f: F) -> ExperimentJob<T>
    where
        F: Fn(&ExperimentRun, T) -> Result<(), Box<dyn Error>> + Send + Sync + 'static,
    {
        let client = self.client.clone();
        let namespace = self.namespace.clone();
        let project = self.project.clone();

        let job_closure = move |input: T| {
            let experiment = Self::setup_experiment::<F>(&client, &namespace, &project)?;

            let handle = experiment.handle();
            let result = handle.in_scope(|| f(&experiment, input));

            match result {
                Ok(()) => experiment
                    .finish()
                    .map_err(|e| format!("Failed to finish experiment: {e}").into()),
                Err(e) => {
                    let msg = e.to_string();
                    let _ = experiment.fail(msg);
                    Err(e)
                }
            }
        };

        ExperimentJob::new(job_closure)
    }

    fn setup_experiment<F>(
        client: &Client,
        namespace: &str,
        project: &str,
    ) -> Result<ExperimentRun, String> {
        let digest = "46523358ec1646354ddab1cd8b93f2b920b44b24a26ea86c129d666d6bae2a5f".to_string();

        let _ = tracel_experiment::integration::tracing::try_init_tracing_subscriber();

        let experiment = ExperimentRun::cloud(
            client.clone(),
            namespace,
            project,
            digest,
            std::any::type_name::<F>().to_string(),
        )
        .map_err(|e| {
            use std::error::Error;
            let mut msg = format!("An error occured while creating the experiment: {e}");
            let mut src = e.source();
            while let Some(s) = src {
                msg.push_str(&format!("caused by: {s}"));
                src = s.source();
            }
            msg
        })?;

        Ok(experiment)
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
