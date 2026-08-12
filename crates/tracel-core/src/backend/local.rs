use std::path::PathBuf;
#[derive(Debug, Clone)]
pub struct LocalBackend {
    pub(crate) path: PathBuf,
}

impl LocalBackend {
    /// Creates an offline backend rooted at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}
