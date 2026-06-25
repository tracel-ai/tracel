#[allow(clippy::module_inception)]
mod cli;
mod error;

pub use cli::Cli;
pub use error::CliError;
