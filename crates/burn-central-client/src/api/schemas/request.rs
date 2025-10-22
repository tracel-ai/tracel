use crate::schemas::{BurnCentralCodeMetadata, CrateVersionMetadata};
use serde::Serialize;

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

#[derive(Debug, Serialize)]
pub struct CodeUploadParamsSchema {
    pub target_package_name: String,
    pub burn_central_metadata: BurnCentralCodeMetadata,
    pub crates: Vec<CrateVersionMetadata>,
    pub digest: String,
}

#[derive(Debug, Serialize)]
pub struct ComputeProviderQueueJobParamsSchema {
    pub compute_provider_group_name: String,
    pub digest: String,
    pub command: String,
}

#[derive(Serialize)]
pub struct CreateProjectSchema {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Serialize)]
pub struct ArtifactFileSpecRequest {
    pub rel_path: String,
    pub size_bytes: u64,
    pub checksum: String,
}

#[derive(Serialize)]
pub struct CreateArtifactRequest {
    pub name: String,
    pub kind: String,
    pub files: Vec<ArtifactFileSpecRequest>,
}

#[derive(Serialize, Default)]
pub struct CompleteUploadRequest {
    pub file_names: Vec<String>,
}

#[derive(Serialize)]
pub struct AddFilesToArtifactRequest {
    pub files: Vec<ArtifactFileSpecRequest>,
}
