use std::path::PathBuf;

use crate::context::{Backend, Context};

#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub(crate) path: PathBuf,
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
}
