use std::path::PathBuf;
use std::{fs, io};

use crate::entity::projects::burn_dir::cache::CacheState;
use crate::entity::projects::burn_dir::project::BurnCentralProject;

pub mod cache;
pub mod project;

pub struct BurnDir {
    root: PathBuf,
}

impl BurnDir {
    pub fn new(root: PathBuf) -> Self {
        BurnDir { root }
    }

    pub fn init(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::write(self.root.join(".gitignore"), "*\n")?;
        Ok(())
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn crates_dir(&self) -> PathBuf {
        self.root.join("crates")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.root.join("artifacts")
    }

    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    pub fn load_cache(&self) -> io::Result<CacheState> {
        CacheState::load(&self.root)
    }

    pub fn save_cache(&self, cache: &CacheState) -> io::Result<()> {
        cache.save(&self.root)
    }

    pub fn load_project(&self) -> io::Result<Option<BurnCentralProject>> {
        BurnCentralProject::load(&self.root)
            .map(Some)
            .or_else(|err| {
                if err.kind() == io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(err)
                }
            })
    }

    pub fn save_project(&self, project: &BurnCentralProject) -> io::Result<()> {
        project.save(&self.root)
    }

    pub fn unlink_project(&self) -> io::Result<()> {
        BurnCentralProject::remove(&self.root)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }
}
