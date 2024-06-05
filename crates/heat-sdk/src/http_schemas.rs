use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct URLSchema {
    pub url: String,
}

#[derive(Serialize)]
pub struct EndStatusSchema {
    pub status: String,
    pub reason: Option<String>,
}

impl EndStatusSchema {
    pub fn Ok() -> Self {
        Self {
            status: "ok".to_string(),
            reason: None,
        }
    }

    pub fn Error(reason: String) -> Self {
        Self {
            status: "err".to_string(),
            reason: Some(reason),
        }
    }
}
