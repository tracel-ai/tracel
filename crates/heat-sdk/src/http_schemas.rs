use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct URLSchema {
    pub url: String,
}

#[derive(Serialize)]
pub enum EndExperimentSchema {
    Success,
    Fail(String)
}