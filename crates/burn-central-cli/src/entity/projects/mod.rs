use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::app_config::Environment;
use crate::entity::projects::burn_dir::{BurnDir, project::BurnCentralProject};
use crate::generation::GeneratedCrate;
use crate::tools::cargo;

pub mod burn_dir;
pub mod project_path;

pub struct ProjectContext {
    pub user_crate_name: String,
    pub user_crate_dir: PathBuf,
    pub generated_crate_name: String,
    pub build_profile: String,
    pub burn_dir: BurnDir,
    pub project: Option<BurnCentralProject>,
    pub metadata: cargo_metadata::Metadata,
}

impl ProjectContext {
    pub fn discover(environment: Environment) -> anyhow::Result<Self> {
        let manifest_path = cargo::try_locate_manifest().ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to locate Cargo.toml in the current directory or any parent directories"
            )
        })?;
        Self::load_from_manifest(&manifest_path, environment)
    }

    pub fn load_from_manifest(
        manifest_path: &Path,
        environment: Environment,
    ) -> anyhow::Result<Self> {
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
        let burn_dir = BurnDir::new(&user_crate_dir, environment);
        burn_dir
            .init()
            .with_context(|| "Burn directory should be initialized")?;

        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(manifest_path)
            .exec()
            .with_context(|| "Failed to load cargo metadata")?;

        let project = burn_dir
            .load_project()
            .with_context(|| "Failed to load project metadata from burn directory")?;

        Ok(Self {
            user_crate_name,
            user_crate_dir,
            generated_crate_name,
            build_profile: "release".to_string(),
            burn_dir,
            project,
            metadata,
        })
    }

    pub fn get_project(&self) -> Option<&BurnCentralProject> {
        self.project.as_ref()
    }

    pub fn save_project(&mut self, project: &BurnCentralProject) -> anyhow::Result<()> {
        self.burn_dir
            .save_project(project)
            .with_context(|| "Failed to save project metadata to burn directory")?;
        self.project = Some(project.clone());
        Ok(())
    }

    pub fn get_workspace_root(&self) -> PathBuf {
        self.metadata.workspace_root.clone().into_std_path_buf()
    }

    pub fn save_crate(&self, generated_crate: GeneratedCrate) -> anyhow::Result<()> {
        let mut cache = self.burn_dir.load_cache()?;
        let name = generated_crate.name();
        generated_crate
            .write_to_burn_dir(&self.burn_dir, &mut cache)
            .with_context(|| {
                format!(
                    "Failed to save generated crate '{}' to burn directory",
                    name
                )
            })?;
        self.burn_dir.save_cache(&cache)?;
        Ok(())
    }

    pub fn burn_dir(&self) -> &BurnDir {
        &self.burn_dir
    }

    pub fn cwd(&self) -> &Path {
        &self.user_crate_dir
    }
}
