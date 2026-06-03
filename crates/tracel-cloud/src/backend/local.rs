use std::path::PathBuf;

use tracel_experiment::ExperimentRun;

use crate::{Backend, Context, DiscoverError};

#[derive(Debug, Clone)]
pub struct LocalBackend {
    path: PathBuf,
}

impl LocalBackend {
    pub fn create_context(path: impl Into<PathBuf>) -> Result<Context, DiscoverError> {
        let local_backend = LocalBackend::new(path.into());
        let backend = Backend::Local(local_backend);
        Ok(Context::new(backend))
    }

    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, String> {
        ExperimentRun::local(self.path.join(routine)).map_err(|e| e.to_string())
    }
}
