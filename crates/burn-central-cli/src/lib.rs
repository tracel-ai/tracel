pub mod cli;
pub mod tools;

mod app_config;
mod commands;
mod context;
mod helpers;
mod logging;

// Re-export library functionality for convenience
pub use burn_central_lib::{
    BurnCentralContext, Client, Config, ProjectContext, Result, compute_provider, config, entity,
    generation, request, response,
};

// Re-export CLI-specific app config functionality
pub use app_config::{AppConfig, Environment};
pub use burn_central_lib::Credentials;

// Re-export specific types that CLI needs
pub use burn_central_lib::context::ClientCreationError;

// Re-export CLI-specific context
pub use context::CliContext;

// Re-export CLI helpers
pub use helpers::project;
