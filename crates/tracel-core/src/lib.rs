mod backend;
mod connection;
mod context;
mod dataset;
mod model_registry;

pub mod experiment;
pub mod inference;

pub use connection::{Connection, ContextError};
pub use context::Context;
pub use dataset::{DatasetError, DatasetModule, DatasetVersionSpec};
pub use model_registry::{ModelRegistryError, ModelRegistryModule};

/// Resolves the base cache directory for downloaded artifacts (models, datasets, ...),
/// falling back to the platform's generic cache dir if a `com.tracel.burncentral`-scoped
/// one isn't available (e.g. missing `$HOME` in a stripped-down container). Returns `None`
/// if neither can be determined.
fn resolve_cache_dir() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("ai", "tracel", "console")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.cache_dir().join("tracel")))
}
