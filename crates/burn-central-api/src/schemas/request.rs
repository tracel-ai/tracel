use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct CreateExperimentSchema {
    pub description: Option<String>,
    pub code_version_digest: String,
    pub routine_run: String,
}

#[derive(Serialize)]
pub struct BurnCentralCredentialsSchema {
    pub api_key: String,
}

pub struct PackagedCrateData {
    pub name: String,
    pub path: PathBuf,
    pub checksum: String,
    pub metadata: serde_json::Value,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisteredFunction {
    pub mod_path: String,
    pub fn_name: String,
    pub proc_type: String,
    pub code: String,
    pub routine: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BurnCentralCodeMetadata {
    pub functions: Vec<RegisteredFunction>,
}

#[derive(Debug, Serialize)]
pub struct CodeUploadParamsSchema {
    pub target_package_name: String,
    pub burn_central_metadata: BurnCentralCodeMetadata,
    pub crates: Vec<CrateVersionMetadata>,
    pub digest: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrateVersionMetadata {
    pub checksum: String,
    pub metadata: serde_json::Value,
    pub size: u64,
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
    pub file_names: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct AddFilesToArtifactRequest {
    pub files: Vec<ArtifactFileSpecRequest>,
}
