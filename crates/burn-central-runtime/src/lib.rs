mod error;
mod executor;
mod inference;
mod input;
mod output;
mod param;
mod routine;
mod type_name;
mod types;

#[cfg(feature = "cli")]
pub mod cli;

pub use error::RuntimeError;
pub use executor::{ExecutionContext, Executor, ExecutorBuilder};
pub use routine::{IntoRoutine, Routine};
pub use types::*;

pub fn setup_logging() {
    env_logger::builder()
        .filter_module("burn_central", log::LevelFilter::Info)
        .filter_module("burn_central_client", log::LevelFilter::Info)
        .filter_module("burn_central_runtime", log::LevelFilter::Info)
        .init();
}
