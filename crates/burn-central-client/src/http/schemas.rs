use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::schemas::{BurnCentralCodeMetadata, CrateVersionMetadata};

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
pub struct BurnCentralCredentialsSchema {
    pub api_key: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct CreateExperimentResponseSchema {
    pub experiment_num: i32,
    pub project_name: String,
    pub status: String,
    pub description: String,
    pub config: serde_json::Value,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CodeUploadParamsSchema {
    pub target_package_name: String,
    pub burn_central_metadata: BurnCentralCodeMetadata,
    pub crates: Vec<CrateVersionMetadata>,
    pub version: String,
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
}

#[allow(dead_code)]
#[derive(Deserialize)]
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
