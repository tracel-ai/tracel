use crate::{
    api::{ModelResponse, ModelVersionResponse},
    schemas::CreatedByUser,
};

/// Information about a specific model version
#[derive(Debug, Clone)]
pub struct ModelVersionInfo {
    pub id: uuid::Uuid,
    pub version: u32,
    pub size: u64,
    pub checksum: String,
    pub created_by: CreatedByUser,
    pub created_at: String,
    pub manifest: serde_json::Value,
    pub experiment_source: Option<ExperimentSource>,
}

impl From<ModelVersionResponse> for ModelVersionInfo {
    fn from(value: ModelVersionResponse) -> Self {
        ModelVersionInfo {
            id: value.id.parse().unwrap_or(uuid::Uuid::nil()),
            version: value.version,
            size: value.size,
            checksum: value.checksum,
            created_by: CreatedByUser {
                id: value.created_by.id,
                username: value.created_by.username,
                namespace: value.created_by.namespace,
            },
            created_at: value.created_at,
            manifest: value.manifest,
            experiment_source: value.experiment.map(|exp| ExperimentSource {
                experiment_id: exp.id,
                experiment_num: exp.experiment_num,
            }),
        }
    }
}

/// Information about the experiment that created a model
#[derive(Debug, Clone)]
pub struct ExperimentSource {
    pub experiment_id: i32,
    pub experiment_num: i32,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub id: uuid::Uuid,
    pub project_id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created_by: CreatedByUser,
    pub created_at: String,
    pub version_count: u64,
}

impl From<ModelResponse> for Model {
    fn from(value: ModelResponse) -> Self {
        Model {
            id: value.id.parse().unwrap_or(uuid::Uuid::nil()),
            project_id: value.project_id,
            name: value.name,
            description: value.description,
            created_by: CreatedByUser {
                id: value.created_by.id,
                username: value.created_by.username,
                namespace: value.created_by.namespace,
            },
            created_at: value.created_at,
            version_count: value.version_count,
        }
    }
}
