use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::schemas::{CrateVersionMetadata, HeatCodeMetadata};

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
    pub heat_metadata: HeatCodeMetadata,
    pub crates: Vec<CrateVersionMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct CodeUploadUrlsSchema {
    pub project_version: u32,
    pub urls: HashMap<String, String>,
}

type RunnerJobCommand = String;

#[derive(Debug, Serialize)]
pub struct RunnerQueueJobParamsSchema {
    pub runner_group_name: String,
    pub project_version: u32,
    pub command: RunnerJobCommand,
}
