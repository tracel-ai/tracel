mod error;
mod routine;
mod param;
mod types;
mod output;
mod executor;
mod type_name;

mod backend;

#[cfg(feature = "cli")]
pub mod cli;

pub use executor::{ExecutorBuilder, Executor, ExecutionContext};
pub use error::RuntimeError;
pub use routine::{IntoRoutine, Routine};
pub use types::*;

pub fn setup_logging() {
    env_logger::builder()
        .filter_module("burn_central", log::LevelFilter::Info)
        .filter_module("burn_central_client", log::LevelFilter::Info)
        .filter_module("burn_central_runtime", log::LevelFilter::Info)
        .init();
}
