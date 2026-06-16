use std::{error::Error, fmt};

#[derive(Debug)]
pub enum CliError {
    UnknownJob { name: String, available: Vec<String> },
    MissingDefault,
    ConfigError(Box<dyn Error + Send + Sync>),
    JobError(Box<dyn Error + Send + Sync>),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnknownJob { name, available } => {
                write!(f, "unknown job '{name}'. Available: {}", available.join(", "))
            }
            CliError::MissingDefault => {
                write!(f, "no job name given and no default registered")
            }
            CliError::ConfigError(e) => write!(f, "invalid config: {e}"),
            CliError::JobError(e) => write!(f, "job failed: {e}"),
        }
    }
}

impl Error for CliError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            CliError::ConfigError(e) | CliError::JobError(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}
