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
