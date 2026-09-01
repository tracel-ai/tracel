use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub path: PathBuf,
}

impl LocalBackend {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
