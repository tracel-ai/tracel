use std::path::{Path, PathBuf};

use crate::entity::projects::burn_dir::{BurnDir, project::BurnCentralProject};

pub mod burn_dir;
pub mod project_path;

pub struct ProjectContext {
    pub user_crate_name: String,
    pub user_crate_dir: PathBuf,
    pub generated_crate_name: String,
    pub build_profile: String,
    pub burn_dir: BurnDir,
    pub project: Option<BurnCentralProject>,
}

impl ProjectContext {
    pub fn load_from_manifest(manifest_path: &Path) -> Self {
        // assert that the manifest path is a file
        assert!(manifest_path.is_file());
        assert!(manifest_path.ends_with("Cargo.toml"));
        // get the project name from the Cargo.toml
        let toml_str = std::fs::read_to_string(manifest_path).expect("Cargo.toml should exist");
        let manifest_document =
            toml::de::from_str::<toml::Value>(&toml_str).expect("Cargo.toml should be valid");

        let user_crate_name = manifest_document["package"]["name"]
            .as_str()
            .expect("Package name should exist")
            .to_string();
        let generated_crate_name = format!("{user_crate_name}_gen");

        let user_crate_dir = manifest_path
            .parent()
            .expect("Project directory should exist")
            .to_path_buf();
        let burn_dir = BurnDir::new(&user_crate_dir);
        burn_dir
            .init()
            .expect("Burn directory should be initialized");

        Self {
            user_crate_name,
            user_crate_dir,
            generated_crate_name,
            build_profile: "release".to_string(),
            burn_dir,
            project: None,
        }
    }

    pub fn load_project(&mut self) -> anyhow::Result<()> {
        self.project = Some(self.burn_dir.load_project()?);
        Ok(())
    }
}
