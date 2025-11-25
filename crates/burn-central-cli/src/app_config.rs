use directories::ProjectDirs;

use std::{fs, io, path::PathBuf};

use crate::context::Credentials;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Development,
}

impl Environment {
    pub fn from_dev_flag(is_dev: bool) -> Self {
        if is_dev {
            Self::Development
        } else {
            Self::Production
        }
    }

    fn file_suffix(&self) -> &'static str {
        match self {
            Self::Production => "prod",
            Self::Development => "dev",
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Missing configuration directory")]
    MissingDirectory,
}

pub struct AppConfig {
    base_dir: PathBuf,
    environment: Environment,
}

impl AppConfig {
    pub fn new(environment: Environment) -> Result<Self, ConfigError> {
        let proj_dirs = ProjectDirs::from("com", "tracel", "burncentral")
            .ok_or(ConfigError::MissingDirectory)?;

        let config_dir = proj_dirs.config_dir().to_path_buf();
        fs::create_dir_all(&config_dir)?; // Ensure it exists

        Ok(Self {
            base_dir: config_dir,
            environment,
        })
    }

    fn credentials_path(&self) -> PathBuf {
        let filename = format!("credentials-{}.json", self.environment.file_suffix());
        self.base_dir.join(filename)
    }

    pub fn save_credentials(&self, creds: &Credentials) -> Result<(), ConfigError> {
        let json = serde_json::to_string_pretty(creds)?;
        fs::write(self.credentials_path(), json)?;
        Ok(())
    }

    pub fn load_credentials(&self) -> Result<Option<Credentials>, ConfigError> {
        let path = self.credentials_path();
        if path.exists() {
            let contents = fs::read_to_string(path)?;
            let creds = serde_json::from_str(&contents)?;
            Ok(Some(creds))
        } else {
            Ok(None)
        }
    }
}
