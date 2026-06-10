use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub(crate) path: PathBuf,
}

impl LocalBackend {
    pub fn create_context(path: impl Into<PathBuf>) -> LocalBackend {
        LocalBackend::new(path.into())
    }

    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}
