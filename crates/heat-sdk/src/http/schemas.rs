use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
pub struct CodeUploadParamsSchema {
    pub project_id: String,
    pub crate_names: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodeUploadUrl {
    pub crate_name: String,
    pub url: String,
}
#[derive(Debug, Deserialize)]
pub struct CodeUploadUrlsSchema {
    pub urls: Vec<CodeUploadUrl>,
}