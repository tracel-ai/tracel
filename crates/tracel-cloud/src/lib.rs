use std::path::PathBuf;

use backend::cloud::CloudBackend;
use burn_central_client::ClientError;
use burn_central_client::Env;
use module::experiment::Experiment;
use module::experiment::RunProvider;
use tracel_experiment::ExperimentRun;

use crate::backend::local::LocalBackend;
use crate::backend::station::StationBackend;

mod backend;
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
}

// make a concrete type for each context backend that implements the module traits, and then Context can just delegate to the backend's implementation. This way we can avoid having a lot of conditional logic in the module implementations about which backend is being used, and instead isolate that logic to the Context struct.
// it also facilitates moving to dynamic dispatch of modules in the future
#[derive(Debug, Clone)]
pub enum Backend {
    Cloud(CloudBackend),
    Local(LocalBackend),
    Station(StationBackend),
}

impl Context {
    fn new(backend: Backend) -> Self {
        Self { backend }
    }

    pub fn cloud(env: Env) -> Result<Self, DiscoverError> {
        CloudBackend::create_context(env)
    }

    pub fn local(path: impl Into<PathBuf>) -> Result<Self, DiscoverError> {
        LocalBackend::create_context(path)
    }

    pub fn experiment(&self) -> Experiment<Context> {
        Experiment::new(self.clone())
    }
}

impl RunProvider for Context {
    fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, String> {
        match self.backend.clone() {
            Backend::Cloud(backend) => backend.setup_experiment(routine),
            Backend::Station(backend) => {
                ExperimentRun::station(backend.client, routine).map_err(|e| {
                    use std::error::Error;
                    let mut msg = format!("An error occured while creating the experiment: {e}");
                    let mut src = e.source();
                    while let Some(s) = src {
                        msg.push_str(&format!("caused by: {s}"));
                        src = s.source();
                    }
                    msg
                })
            }
            Backend::Local(backend) => backend.setup_experiment(routine),
        }
    }
}
