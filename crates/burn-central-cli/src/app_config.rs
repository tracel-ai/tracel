use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("Missing configuration directory")]
    MissingDirectory,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Credentials {
    pub api_key: String,
}

pub struct AppConfig {
    base_dir: PathBuf,
}

impl AppConfig {
    pub fn new() -> Result<Self, ConfigError> {
        let proj_dirs = ProjectDirs::from("com", "tracel", "burncentral")
            .ok_or(ConfigError::MissingDirectory)?;

        let config_dir = proj_dirs.config_dir().to_path_buf();
        fs::create_dir_all(&config_dir)?; // Ensure it exists

        Ok(Self {
            base_dir: config_dir,
        })
    }

    fn credentials_path(&self) -> PathBuf {
        self.base_dir.join("credentials.json")
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
