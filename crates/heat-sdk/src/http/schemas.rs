use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::schemas::CrateMetadata;

#[derive(Deserialize)]
pub struct URLSchema {
    pub url: String,
}

#[derive(Serialize)]
pub enum EndExperimentSchema {
    Success,
    Fail(String),
}

#[derive(Serialize)]
pub struct StartExperimentSchema {
    pub config: serde_json::Value,
}

#[derive(Serialize)]
pub struct HeatCredentialsSchema {
    pub api_key: String,
}

#[derive(Deserialize)]
pub struct CreateExperimentResponseSchema {
    pub experiment_id: String,
}

#[derive(Debug, Serialize)]
pub struct CodeUploadParamsSchema {
    pub root_crate_name: String,
    pub crates: Vec<CrateMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct CodeUploadUrl {
    pub crate_name: String,
    pub url: String,
}
#[derive(Debug, Deserialize)]
pub struct CodeUploadUrlsSchema {
    pub project_version: u32,
    pub urls: Vec<CodeUploadUrl>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunnerJobCommand {
    pub command: String,
}

#[derive(Debug, Serialize)]
pub struct RunnerQueueJobParamsSchema {
    pub project_id: Uuid,
    pub project_version: u32,
    pub target_package: String,
    pub command: RunnerJobCommand,
}
