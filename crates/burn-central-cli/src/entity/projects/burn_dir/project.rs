use serde::{Deserialize, Serialize};
use std::path::Path;
use std::{fs, io};

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct BurnCentralProject {
    pub name: String,
    pub owner: String,
}

impl BurnCentralProject {
    const BURN_PROJECT_FILENAME: &'static str = "project.toml";

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let contents =
            toml::to_string(&self).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(dir.join(Self::BURN_PROJECT_FILENAME), contents)
    }

    pub fn load(dir: &Path) -> io::Result<Self> {
        let path = dir.join(Self::BURN_PROJECT_FILENAME);
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Project metadata file not found",
            ));
        }
        let contents = fs::read_to_string(path)?;
        let meta =
            toml::from_str(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(meta)
    }

    pub fn remove(dir: &Path) -> io::Result<()> {
        fs::remove_file(dir.join(Self::BURN_PROJECT_FILENAME))
    }
}
