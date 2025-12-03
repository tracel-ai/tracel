use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::entity::projects::burn_dir::{BurnDir, project::BurnCentralProject};
use crate::execution::cancellable::CancellationToken;
use crate::tools::function_discovery::{FunctionDiscovery, FunctionMetadata};
use crate::tools::functions_registry::FunctionRegistry;

pub mod burn_dir;
pub mod project_path;

#[derive(Debug)]
pub enum ErrorKind {
    ManifestNotFound,
    Parsing,
    InvalidPackage,
    BurnDirInitialization,
    BurnDirNotInitialized,
    Unexpected,
}

#[derive(thiserror::Error, Debug)]
pub struct ProjectContextError {
    message: String,
    kind: ErrorKind,
    #[source]
    source: Option<anyhow::Error>,
}

impl ProjectContextError {
    pub fn new(message: String, kind: ErrorKind, source: Option<anyhow::Error>) -> Self {
        Self {
            message,
            kind,
            source,
        }
    }

    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    pub fn is_burn_dir_not_initialized(&self) -> bool {
        matches!(self.kind, ErrorKind::BurnDirNotInitialized)
    }
}

impl std::fmt::Display for ProjectContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub struct ProjectContext {
    pub crate_info: CrateInfo,
    pub build_profile: String,
    pub burn_dir: BurnDir,
    pub project: BurnCentralProject,
    function_registry: Mutex<Vec<FunctionMetadata>>,
}

pub struct CrateInfo {
    pub user_crate_name: String,
    pub user_crate_dir: PathBuf,
    pub metadata: cargo_metadata::Metadata,
}

impl CrateInfo {
    pub fn load_from_path(manifest_path: &Path) -> Result<Self, ProjectContextError> {
        if !manifest_path.is_file() {
            return Err(ProjectContextError::new(
                format!(
                    "Cargo.toml not found at specified path '{}'",
                    manifest_path.display()
                ),
                ErrorKind::ManifestNotFound,
                None,
            ));
        }
        let toml_str = std::fs::read_to_string(manifest_path).expect("Cargo.toml should exist");
        let manifest_document = toml::de::from_str::<toml::Value>(&toml_str).map_err(|e| {
            ProjectContextError::new(
                format!(
                    "Failed to parse Cargo.toml at '{}': {}",
                    manifest_path.display(),
                    e
                ),
                ErrorKind::Parsing,
                Some(anyhow::anyhow!(e)),
            )
        })?;

        if manifest_document.get("package").is_none() {
            return Err(ProjectContextError::new(
                format!(
                    "Cargo.toml at '{}' does not include a [package] section",
                    manifest_path.display()
                ),
                ErrorKind::InvalidPackage,
                None,
            ));
        }

        let user_crate_name = manifest_document["package"]["name"]
            .as_str()
            .expect("Package name should exist")
            .to_string();

        let user_crate_dir = manifest_path
            .parent()
            .expect("Project directory should exist")
            .to_path_buf();

        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(manifest_path)
            .no_deps()
            .exec()
            .map_err(|e| {
                ProjectContextError::new(
                    format!(
                        "Failed to load cargo metadata for manifest at '{}': {}",
                        manifest_path.display(),
                        e
                    ),
                    ErrorKind::Parsing,
                    Some(anyhow::anyhow!(e)),
                )
            })?;

        let package = metadata
            .packages
            .iter()
            .find(|pkg| pkg.name.to_string() == user_crate_name)
            .ok_or_else(|| {
                ProjectContextError::new(
                    format!(
                        "Failed to find package '{}' in cargo metadata",
                        user_crate_name
                    ),
                    ErrorKind::InvalidPackage,
                    None,
                )
            })?;

        // ensure that the package has a lib target
        package
            .targets
            .iter()
            .find(|target| target.kind.contains(&cargo_metadata::TargetKind::Lib))
            .ok_or_else(|| {
                ProjectContextError::new(
                    format!(
                        "Package '{}' does not have a library target",
                        user_crate_name
                    ),
                    ErrorKind::InvalidPackage,
                    None,
                )
            })?;

        Ok(CrateInfo {
            user_crate_name,
            user_crate_dir,
            metadata,
        })
    }

    pub fn get_ws_root(&self) -> PathBuf {
        self.metadata.workspace_root.clone().into_std_path_buf()
    }

    pub fn get_manifest_path(&self) -> PathBuf {
        self.user_crate_dir.join(PathBuf::from("Cargo.toml"))
    }
}

impl ProjectContext {
    pub fn load_crate_info(manifest_path: &Path) -> Result<CrateInfo, ProjectContextError> {
        CrateInfo::load_from_path(manifest_path)
    }

    pub fn load(manifest_path: &Path, burn_dir_name: &str) -> Result<Self, ProjectContextError> {
        let crate_info = CrateInfo::load_from_path(manifest_path)?;
        let burn_dir_root = crate_info.user_crate_dir.join(PathBuf::from(burn_dir_name));
        let burn_dir = BurnDir::new(burn_dir_root);
        burn_dir.init().map_err(|e| {
            ProjectContextError::new(
                "Failed to initialize Burn directory".to_string(),
                ErrorKind::BurnDirNotInitialized,
                Some(e.into()),
            )
        })?;

        let project = burn_dir
            .load_project()
            .map_err(|e| {
                ProjectContextError::new(
                    "Failed to load project metadata from Burn directory".to_string(),
                    ErrorKind::BurnDirNotInitialized,
                    Some(e.into()),
                )
            })?
            .ok_or_else(|| {
                ProjectContextError::new(
                    "No Burn Central project linked to this repository".to_string(),
                    ErrorKind::BurnDirNotInitialized,
                    None,
                )
            })?;

        Ok(Self {
            crate_info,
            build_profile: "release".to_string(),
            burn_dir,
            project,
            function_registry: Mutex::new(Default::default()),
        })
    }

    pub fn init(
        project: BurnCentralProject,
        manifest_path: &Path,
        burn_dir_name: &str,
    ) -> Result<Self, ProjectContextError> {
        let crate_info = CrateInfo::load_from_path(manifest_path)?;

        let burn_dir_root = crate_info.user_crate_dir.join(PathBuf::from(burn_dir_name));
        let burn_dir = BurnDir::new(burn_dir_root);
        burn_dir.init().map_err(|e| {
            ProjectContextError::new(
                "Failed to initialize Burn directory".to_string(),
                ErrorKind::BurnDirInitialization,
                Some(e.into()),
            )
        })?;

        burn_dir.save_project(&project).map_err(|e| {
            ProjectContextError::new(
                "Failed to save project metadata to Burn directory".to_string(),
                ErrorKind::BurnDirInitialization,
                Some(e.into()),
            )
        })?;

        Ok(Self {
            crate_info,
            build_profile: "release".to_string(),
            burn_dir,
            project: project.clone(),
            function_registry: Mutex::new(Default::default()),
        })
    }

    pub fn unlink(manifest_path: &Path, burn_dir_name: &str) -> Result<(), ProjectContextError> {
        let crate_info = CrateInfo::load_from_path(manifest_path)?;

        let burn_dir_root = crate_info.user_crate_dir.join(PathBuf::from(burn_dir_name));
        let burn_dir = BurnDir::new(burn_dir_root);

        std::fs::remove_dir_all(burn_dir.root()).map_err(|e| {
            ProjectContextError::new(
                "Failed to remove Burn directory".to_string(),
                ErrorKind::Unexpected,
                Some(e.into()),
            )
        })?;

        Ok(())
    }

    pub fn get_project(&self) -> &BurnCentralProject {
        &self.project
    }

    pub fn get_crate_name(&self) -> &str {
        &self.crate_info.user_crate_name
    }

    pub fn get_crate_path(&self) -> &Path {
        &self.crate_info.user_crate_dir
    }

    pub fn get_workspace_root(&self) -> PathBuf {
        self.crate_info
            .metadata
            .workspace_root
            .clone()
            .into_std_path_buf()
    }

    pub fn get_manifest_path(&self) -> PathBuf {
        self.crate_info.get_manifest_path()
    }

    pub fn burn_dir(&self) -> &BurnDir {
        &self.burn_dir
    }

    pub fn cwd(&self) -> &Path {
        &self.crate_info.user_crate_dir
    }

    pub fn load_functions(&self) -> anyhow::Result<FunctionRegistry> {
        let token = CancellationToken::new();
        self.load_functions_cancellable(&token)
    }

    pub fn load_functions_cancellable(
        &self,
        cancel_token: &CancellationToken,
    ) -> anyhow::Result<FunctionRegistry> {
        use crate::execution::cancellable::check_cancelled_anyhow;

        let mut functions = self.function_registry.lock().unwrap();
        if functions.is_empty() {
            check_cancelled_anyhow!(cancel_token, "Function loading was cancelled");

            let current_pkg = self.get_current_package();
            let discovered_functions = FunctionDiscovery::new(&self.crate_info.user_crate_dir)
                .with_manifest_path(current_pkg.manifest_path.clone())
                .with_target_dir(self.burn_dir.target_dir())
                .discover_functions(cancel_token)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to discover functions in crate '{}': {}",
                        self.crate_info.user_crate_name,
                        e
                    )
                })?;
            *functions = discovered_functions;
        }
        Ok(FunctionRegistry::new(functions.clone()))
    }

    pub fn get_current_package(&self) -> &cargo_metadata::Package {
        self.crate_info
            .metadata
            .packages
            .iter()
            .find(|pkg| pkg.name.to_string() == self.crate_info.user_crate_name)
            .expect("Current package should be found in metadata")
    }
}
