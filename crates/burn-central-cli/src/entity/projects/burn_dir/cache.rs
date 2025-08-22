use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::{fs, io, path::Path};
use toml;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CrateEntry {
    pub path: String,
    pub created_at: String,
    pub hash: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BinaryEntry {
    pub filename: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CacheState {
    pub schema_version: String,
    pub crates: HashMap<String, CrateEntry>,
    pub binaries: HashMap<String, BinaryEntry>,
}

impl CacheState {
    const BURN_CACHE_FILENAME: &'static str = "cache.toml";

    pub fn load(dir: &Path) -> io::Result<Self> {
        let path = dir.join(Self::BURN_CACHE_FILENAME);
        if !path.exists() {
            return Ok(CacheState {
                schema_version: "1".to_string(),
                ..Default::default()
            });
        }
        let contents = fs::read_to_string(path)?;
        let parsed =
            toml::from_str(&contents).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(parsed)
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let contents = toml::to_string_pretty(self).map_err(io::Error::other)?;
        fs::create_dir_all(dir)?;
        fs::write(dir.join(Self::BURN_CACHE_FILENAME), contents)
    }

    pub fn add_crate(&mut self, name: &str, path: String, hash: String) {
        self.crates.insert(
            name.to_string(),
            CrateEntry {
                path,
                hash,
                created_at: Utc::now().to_rfc3339(),
            },
        );
    }

    pub fn get_crate(&self, name: &str) -> Option<&CrateEntry> {
        self.crates.get(name)
    }

    pub fn remove_crate(&mut self, name: &str) -> Option<CrateEntry> {
        self.crates.remove(name)
    }

    pub fn add_binary(&mut self, name: &str, filename: String) {
        self.binaries.insert(
            name.to_string(),
            BinaryEntry {
                filename,
                created_at: Utc::now().to_rfc3339(),
            },
        );
    }

    #[allow(dead_code)]
    pub fn get_binary(&self, name: &str) -> Option<&BinaryEntry> {
        self.binaries.get(name)
    }

    #[allow(dead_code)]
    pub fn remove_binary(&mut self, name: &str) -> Option<BinaryEntry> {
        self.binaries.remove(name)
    }
}
