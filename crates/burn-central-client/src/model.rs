use crate::api::{Client, ClientError};
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelRegistryError {
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Invalid model specification: {0}")]
    InvalidModelSpec(String),
    #[error("Client error: {0}")]
    Client(#[from] ClientError),
}

pub struct ModelArtifact {
    namespace: String,
    project_name: String,

    name: String,
    version: String,
    description: Option<String>,

    weights_path: Option<PathBuf>,
    config: serde_json::Value,

    client: Client,
}

impl ModelArtifact {
    fn new(
        namespace: String,
        project_name: String,
        name: String,
        version: String,
        description: Option<String>,
        weights_path: Option<PathBuf>,
        config: serde_json::Value,
        client: Client,
    ) -> Self {
        ModelArtifact {
            namespace,
            project_name,
            name,
            version,
            description,
            weights_path,
            config,
            client,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn download(&self, destination: &PathBuf) -> Result<(), String> {
        // stub implementation
        Ok(())
    }

    pub fn get_weights(&self) -> Option<Vec<u8>> {
        if let Some(path) = &self.weights_path {
            std::fs::read(path).ok()
        } else {
            None
        }
    }

    pub fn get_config(&self) -> &serde_json::Value {
        &self.config
    }
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub name: String,
    pub version: u32,
}

impl Display for ModelSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.name, self.version)
    }
}

impl ModelSpec {
    pub fn new(name: String, version: u32) -> Self {
        ModelSpec { name, version }
    }
}

impl FromStr for ModelSpec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err("Invalid model spec format. Expected 'name:version'.".to_string());
        }
        let name = parts[0].to_string();
        let version = parts[1]
            .parse::<u32>()
            .map_err(|_| "Version must be a valid integer.".to_string())?;
        Ok(ModelSpec { name, version })
    }
}

/// A registry in Burn Central that holds models and their metadata.
pub struct ModelRegistry {
    client: Client,
    namespace: String,
    project_name: String,
}

impl ModelRegistry {
    pub fn new(client: Client, namespace: String, project_name: String) -> Self {
        ModelRegistry {
            client,
            namespace,
            project_name,
        }
    }

    pub fn get_model(&self, spec: ModelSpec) -> Result<ModelArtifact, ModelRegistryError> {
        // stub implementation
        Ok(ModelArtifact::new(
            self.namespace.clone(),
            self.project_name.clone(),
            spec.name,
            spec.version.to_string(),
            None,
            None,
            serde_json::json!({}),
            self.client.clone(),
        ))
    }
}
