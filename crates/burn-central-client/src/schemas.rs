use std::{collections::BTreeMap, path::PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::errors::client::BurnCentralClientError;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DepKind {
    Dev,
    Build,
    Normal,
}

#[allow(unused, clippy::too_many_arguments)]
#[derive(Debug, Serialize, Deserialize, derive_new::new)]
pub struct Dep {
    /// Name of the dependency. If the dependency is renamed from the
    /// original package name, this is the original name. The new package
    /// name is stored in the `explicit_name_in_toml` field.
    name: String,
    /// The semver requirement for this dependency.
    version_req: String,
    /// Array of features (as strings) enabled for this dependency.
    features: Vec<String>,
    /// Boolean of whether or not this is an optional dependency.
    optional: bool,
    /// Boolean of whether or not default features are enabled.
    default_features: bool,
    /// The target platform for the dependency. Null if not a target
    /// dependency. Otherwise, a string such as "cfg(windows)".
    target: Option<String>,
    /// The dependency kind.
    kind: DepKind,
    /// The URL of the index of the registry where this dependency is from
    /// as a string. If not specified or null, it is assumed the
    /// dependency is in the current registry.
    registry: Option<String>,
    /// If the dependency is renamed, this is a string of the new package
    /// name. If not specified or null, this dependency is not renamed.
    explicit_name_in_toml: Option<String>,
}

#[allow(unused, clippy::too_many_arguments)]
#[derive(Debug, Serialize, Deserialize, derive_new::new, Default)]
pub struct CrateMetadata {
    /// The name of the package.
    name: String,
    /// The version of the package being published.
    vers: String,
    /// Array of direct dependencies of the package.
    deps: Vec<Dep>,
    /// Set of features defined for the package. Each feature maps to an
    /// array of features or dependencies it enables. Cargo does not
    /// impose limitations on feature names, but crates.io requires
    /// alphanumeric ASCII, '_' or '-' characters.
    features: BTreeMap<String, Vec<String>>,
    /// List of strings of the authors.
    /// May be empty. crates.io requires at least one entry.
    authors: Vec<String>,
    /// Description field from the manifest. May be null. crates.io
    /// requires at least some content.
    description: Option<String>,
    /// String of the URL to the website for this package's documentation.
    /// May be null.
    documentation: Option<String>,
    /// String of the URL to the website for this package's home page. May
    /// be null.
    homepage: Option<String>,
    /// String of the content of the README file. May be null.
    readme: Option<String>,
    /// String of a relative path to a README file in the crate.
    /// May be null.
    readme_file: Option<String>,
    /// Array of strings of keywords for the package.
    keywords: Vec<String>,
    /// Array of strings of categories for the package.
    categories: Vec<String>,
    /// String of the license for the package. May be null. crates.io
    /// requires either `license` or `license_file` to be set.
    license: Option<String>,
    /// String of a relative path to a license file in the crate. May be
    /// null.
    license_file: Option<String>,
    /// String of the URL to the website for the source repository of this
    /// package. May be null.
    repository: Option<String>,
    /// Optional object of "status" badges. Each value is an object of
    /// arbitrary string to string mappings. crates.io has special
    /// interpretation of the format of the badges.
    badges: BTreeMap<String, BTreeMap<String, String>>,
    /// The `links` string value from the package's manifest, or null if
    /// not specified. This field is optional and defaults to null.
    links: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CrateData {
    pub metadata: CrateMetadata,
    pub data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrateVersionMetadata {
    pub checksum: String,
    pub metadata: CrateMetadata,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisteredFunction {
    pub mod_path: String,
    pub fn_name: String,
    pub proc_type: String,
    pub code: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BurnCentralCodeMetadata {
    pub functions: Vec<RegisteredFunction>,
}

pub struct PackagedCrateData {
    pub name: String,
    pub path: PathBuf,
    pub checksum: String,
    pub metadata: CrateMetadata,
}

#[derive(Debug, Clone)]
pub struct ProjectPath {
    owner_name: String,
    project_name: String,
}

impl ProjectPath {
    pub fn new(owner_name: String, project_name: String) -> Self {
        ProjectPath {
            owner_name,
            project_name,
        }
    }

    pub fn validate_path(path: &str) -> bool {
        static NAME_REGEX: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^[a-zA-Z0-9_.-]+$")
                .expect("Should be able to compile name validation regex.")
        });

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 2 {
            return false;
        }

        for part in parts {
            if !NAME_REGEX.is_match(part) {
                return false;
            }
        }

        true
    }

    pub fn owner_name(&self) -> &str {
        &self.owner_name
    }

    pub fn project_name(&self) -> &str {
        &self.project_name
    }
}

impl TryFrom<String> for ProjectPath {
    type Error = BurnCentralClientError;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !ProjectPath::validate_path(&path) {
            return Err(BurnCentralClientError::InvalidProjectPath(path));
        }

        let parts: Vec<&str> = path.split('/').collect();
        Ok(ProjectPath {
            owner_name: parts[0].into(),
            project_name: parts[1].into(),
        })
    }
}

impl std::fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner_name, self.project_name)
    }
}

#[derive(Debug, Clone)]
pub struct ExperimentPath {
    project_path: ProjectPath,
    experiment_num: i32,
}

impl ExperimentPath {
    pub fn validate_path(path: &str) -> bool {
        static NAME_REGEX: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^[a-zA-Z0-9_.-]+$")
                .expect("Should be able to compile name validation regex.")
        });

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() != 3 {
            return false;
        }

        if !NAME_REGEX.is_match(parts[0]) || !NAME_REGEX.is_match(parts[1]) {
            return false;
        }

        if parts[2].parse::<i32>().is_err() {
            return false;
        }

        true
    }

    pub fn owner_name(&self) -> &str {
        &self.project_path.owner_name
    }

    pub fn project_name(&self) -> &str {
        &self.project_path.project_name
    }

    pub fn experiment_num(&self) -> i32 {
        self.experiment_num
    }
}

impl TryFrom<String> for ExperimentPath {
    type Error = BurnCentralClientError;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !ExperimentPath::validate_path(&path) {
            return Err(BurnCentralClientError::InvalidExperimentPath(path));
        }

        let parts: Vec<&str> = path.split('/').collect();
        let project_path = ProjectPath::try_from(parts[0..2].join("/"))?;
        let experiment_num = parts[2]
            .parse::<i32>()
            .map_err(|_| BurnCentralClientError::InvalidExperimentNumber(parts[2].to_string()))?;

        Ok(ExperimentPath {
            project_path,
            experiment_num,
        })
    }
}

impl std::fmt::Display for ExperimentPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.project_path, self.experiment_num)
    }
}

#[derive(Debug, Clone)]
pub struct Experiment {
    pub experiment_num: i32,
    pub project_name: String,
    pub status: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_by: String,
    pub created_at: String,
}

pub struct User {
    pub username: String,
    pub email: String,
}
