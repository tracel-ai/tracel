mod backend;
mod cloud;
mod connection;
mod context;

pub mod experiment;
pub mod inference;

pub use connection::{Connection, ContextError};
pub use context::Context;

use directories::{BaseDirs, ProjectDirs};
use std::path::PathBuf;

pub(crate) fn resolve_config_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "tracel")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .or_else(|| BaseDirs::new().map(|dirs| dirs.config_dir().join("tracel")))
}
