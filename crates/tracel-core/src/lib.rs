mod backend;
mod context;
mod dataset;
mod model_registry;

pub mod experiment;
pub mod inference;

pub use backend::cloud::{AuthMethod, CloudBackend, CloudError, CloudSession, authenticate};
pub use backend::local::LocalBackend;
#[cfg(feature = "station")]
pub use backend::station::{StationBackend, StationError};
pub use context::{Context, ContextError};
pub use dataset::{
    DatasetError, DatasetItem, DatasetModule, DatasetVersionSpec, DownloadedDataset,
    StreamedDataset,
};
pub use model_registry::{ModelRegistryError, ModelRegistryModule};

use directories::{BaseDirs, ProjectDirs};
use std::path::PathBuf;

fn resolve_cache_dir() -> Option<std::path::PathBuf> {
    ProjectDirs::from("", "", "tracel")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.cache_dir().join("tracel")))
}

pub(crate) fn resolve_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "tracel")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .or_else(|| BaseDirs::new().map(|dirs| dirs.config_dir().join("tracel")))
}
