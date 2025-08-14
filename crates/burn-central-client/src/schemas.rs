use crate::client::BurnCentralError;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use burn_central_package::CrateMetadata;

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

#[derive(Debug, Clone)]
pub struct ProjectPath {
    pub owner_name: String,
    pub project_name: String,
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
    type Error = BurnCentralError;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !ProjectPath::validate_path(&path) {
            return Err(Self::Error::InvalidProjectPath(path));
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
    type Error = BurnCentralError;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !ExperimentPath::validate_path(&path) {
            return Err(Self::Error::InvalidExperimentPath(path));
        }

        let parts: Vec<&str> = path.split('/').collect();
        let project_path = ProjectPath::try_from(parts[0..2].join("/"))?;
        let experiment_num = parts[2]
            .parse::<i32>()
            .map_err(|_| Self::Error::InvalidExperimentNumber(parts[2].to_string()))?;

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
    pub namespace: String,
}

#[derive(Debug, Clone)]
pub struct ProjectSchema {
    pub project_name: String,
    pub namespace_name: String,
    pub namespace_type: String,
    pub description: String,
    pub created_by: String,
    pub created_at: String,
    pub visibility: String,
}
