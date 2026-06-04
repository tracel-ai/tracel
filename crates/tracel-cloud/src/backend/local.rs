use std::path::PathBuf;

use tracel_experiment::ExperimentRun;

use tracel_experiment::error::ExperimentError;

use crate::{Backend, Context};

#[derive(Debug, Clone)]
pub struct LocalBackend {
    path: PathBuf,
}

impl LocalBackend {
    pub fn create_context(path: impl Into<PathBuf>) -> Context {
        let local_backend = LocalBackend::new(path.into());
        let backend = Backend::Local(local_backend);
        Context::new(backend)
    }

    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn setup_experiment(&self, routine: String) -> Result<ExperimentRun, ExperimentError> {
        ExperimentRun::local(self.path.join(routine))
    }
}
