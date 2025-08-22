use std::collections::HashMap;

use crate::schemas::{BurnCentralCodeMetadata, CrateVersionMetadata};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
pub struct CreateExperimentSchema {
    pub description: Option<String>,
    pub config: serde_json::Value,
    pub code_version_digest: String,
    pub routine_run: String,
}

#[derive(Serialize)]
pub struct BurnCentralCredentialsSchema {
    pub api_key: String,
}

#[derive(Deserialize)]
pub struct ExperimentResponse {
    pub id: i32,
    pub experiment_num: i32,
    pub project_id: i32,
    pub status: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_by: CreatedByUserResponse,
    pub created_at: String,
    pub code_version_id: Option<Uuid>,
}

#[derive(Deserialize)]
pub struct CreatedByUserResponse {
    pub id: i32,
    pub username: String,
    pub namespace: String,
}

#[derive(Debug, Serialize)]
pub struct CodeUploadParamsSchema {
    pub target_package_name: String,
    pub burn_central_metadata: BurnCentralCodeMetadata,
    pub crates: Vec<CrateVersionMetadata>,
    pub digest: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeUploadUrlsSchema {
    pub project_version: String,
    pub urls: HashMap<String, String>,
}

type RunnerJobCommand = String;

#[derive(Debug, Serialize)]
pub struct RunnerQueueJobParamsSchema {
    pub runner_group_name: String,
    pub project_version: String,
    pub command: RunnerJobCommand,
}

#[derive(Deserialize)]
pub struct UserResponseSchema {
    #[serde(rename = "id")]
    pub _id: i32,
    pub username: String,
    pub email: String,
    pub namespace: String,
}

#[derive(Deserialize, Debug)]
pub struct ProjectSchema {
    pub project_name: String,
    pub namespace_name: String,
    pub namespace_type: String,
    pub description: String,
    pub created_by: String,
    pub created_at: String,
    pub visibility: String,
}

#[derive(Serialize)]
pub struct CreateProjectSchema {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct GetUserOrganizationsResponseSchema {
    pub organizations: Vec<OrganizationSchema>,
}

#[derive(Deserialize)]
pub struct OrganizationSchema {
    pub id: i32,
    pub name: String,
    pub namespace: String,
}
