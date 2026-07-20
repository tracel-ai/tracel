/// Errors surfaced by the model registry.
#[derive(Debug, thiserror::Error)]
pub enum ModelRegistryError {
    #[error("model '{name}' not found")]
    ModelNotFound { name: String },
    #[error("version {version} of model '{name}' not found")]
    VersionNotFound { name: String, version: u32 },
    /// The version's metadata is known but its artifact bytes are not locally present (e.g. a
    /// synced version that has not been pulled). Distinct from [`Self::VersionNotFound`] (the
    /// metadata itself is absent) and [`Self::Backend`] (a real failure). Materializing an
    /// absent version is an explicit engine operation, not a `fetch` side effect.
    #[error("artifacts of version {version} of model '{name}' are not available locally")]
    ArtifactsUnavailable { name: String, version: u32 },
    #[error("artifact error: {0}")]
    Artifact(String),
    /// A backend failure (local store, cloud, station). The registry does not model transport.
    #[error(transparent)]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}
