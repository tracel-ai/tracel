use crate::api::{Client, ClientError};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fmt::Display;
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

#[derive(Debug, Clone)]
pub struct ModelArtifact {
    manifest: ModelManifest,
    client: Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelManifest {
    pub id: uuid::Uuid, // concrete version id on server
    pub name: String,
    pub namespace: String,
    pub project: String,
    pub version: u32,
    pub description: Option<String>,
    pub created_at: String,
    pub config: serde_json::Value,
    pub digest: String,
    pub size: usize,
}

impl ModelArtifact {
    pub fn download(&self, writer: &mut impl std::io::Write) -> Result<(), ClientError> {
        Ok(())
    }

    pub fn get_config(&self) -> &serde_json::Value {
        &self.manifest.config
    }
}

#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub namespace: String,
    pub project_name: String,
    pub name: String,
    pub version: u32,
}

impl Display for ModelSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}:{}",
            self.namespace, self.project_name, self.name, self.version
        )
    }
}

impl ModelSpec {
    pub fn new(namespace: String, project_name: String, name: String, version: u32) -> Self {
        ModelSpec {
            namespace,
            project_name,
            name,
            version,
        }
    }
}

impl FromStr for ModelSpec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('/').collect();
        if parts.len() != 3 {
            return Err("Invalid model spec format".to_string());
        }
        let namespace = parts[0].to_string();
        let project_name = parts[1].to_string();
        let name_version: Vec<&str> = parts[2].split(':').collect();
        if name_version.len() != 2 {
            return Err("Invalid model name and version format".to_string());
        }
        let name = name_version[0].to_string();
        let version = name_version[1]
            .parse::<u32>()
            .map_err(|_| "Invalid version number".to_string())?;
        Ok(ModelSpec {
            namespace,
            project_name,
            name,
            version,
        })
    }
}

/// A registry in Burn Central that holds models and their metadata.
pub struct ModelRegistry {
    client: Client,
}

impl ModelRegistry {
    pub fn new(client: Client) -> Self {
        ModelRegistry { client }
    }

    pub fn get_model(&self, spec: ModelSpec) -> Result<ModelArtifact, ModelRegistryError> {
        // stub implementation
        let manifest = ModelManifest {
            id: uuid::Uuid::new_v4(),
            name: spec.name,
            namespace: spec.namespace,
            project: spec.project_name.clone(),
            version: spec.version,
            description: Some("A sample model".to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            config: serde_json::json!({ "param": "value" }),
            digest: "dummy-digest".to_string(),
            size: 123456,
        };
        Ok(ModelArtifact {
            manifest,
            client: self.client.clone(),
        })
    }
}
