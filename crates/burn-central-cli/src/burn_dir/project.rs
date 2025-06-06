use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct ProjectMeta {
    pub name: String,
    pub schema_version: String,
    pub default_profile: Option<String>,
    pub backend: Option<String>,
}