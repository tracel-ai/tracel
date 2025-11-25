use std::str::FromStr;

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectPath {
    pub owner_name: String,
    pub project_name: String,
}

impl ProjectPath {
    pub fn new(owner_name: impl Into<String>, project_name: impl Into<String>) -> Self {
        ProjectPath {
            owner_name: owner_name.into(),
            project_name: project_name.into(),
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
    type Error = anyhow::Error;

    fn try_from(path: String) -> Result<Self, Self::Error> {
        if !ProjectPath::validate_path(&path) {
            anyhow::bail!("Invalid project path: {}", path);
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

impl FromStr for ProjectPath {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ProjectPath::try_from(s.to_string())
    }
}
