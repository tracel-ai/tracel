use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct URLSchema {
    pub url: String,
}

#[derive(Deserialize)]
pub struct ExperimentResponse {
    pub experiment_num: i32,
}

#[derive(Deserialize)]
pub struct CreatedByUserResponse {
    pub id: i32,
    pub username: String,
    pub namespace: String,
}

#[derive(Debug, Deserialize)]
pub struct CodeUploadUrlsSchema {
    pub id: String,
    pub digest: String,
    pub urls: Option<HashMap<String, String>>,
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
}

#[derive(Deserialize)]
pub struct GetUserOrganizationsResponseSchema {
    pub organizations: Vec<OrganizationSchema>,
}

#[derive(Deserialize)]
pub struct OrganizationSchema {
    pub name: String,
    pub namespace: String,
}

#[derive(Deserialize)]
pub struct PresignedUploadUrlResponse {
    pub part: u32,
    pub url: String,
    pub size_bytes: u64,
}

#[derive(Deserialize)]
pub struct MultipartUploadReponse {
    pub id: String,
    pub parts: Vec<PresignedUploadUrlResponse>,
}

#[derive(Deserialize)]
pub struct PresignedArtifactFileUploadUrlsResponse {
    pub rel_path: String,
    pub urls: MultipartUploadReponse,
}

#[derive(Deserialize)]
pub struct ArtifactCreationResponse {
    pub id: String,
    pub files: Vec<PresignedArtifactFileUploadUrlsResponse>,
}

#[derive(Deserialize)]
pub struct ArtifactAddFileResponse {
    pub files: Vec<PresignedArtifactFileUploadUrlsResponse>,
}

#[derive(Deserialize)]
pub struct PresignedArtifactFileUrlResponse {
    pub rel_path: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct ArtifactDownloadResponse {
    pub files: Vec<PresignedArtifactFileUrlResponse>,
}

#[derive(Deserialize)]
pub struct ArtifactResponse {
    pub id: String,
    pub created_at: String,
    pub name: String,
    pub kind: String,
    pub bucket_id: String,
    pub experiment: ExperimentSourceResponse,
    pub manifest: serde_json::Value,
}

#[derive(Deserialize)]
pub struct ArtifactListResponse {
    pub items: Vec<ArtifactResponse>,
    pub total: usize,
}

#[derive(Deserialize)]
pub struct ExperimentSourceResponse {
    pub id: i32,
    pub experiment_num: i32,
}

#[derive(Deserialize)]
pub struct ModelDownloadResponse {
    pub files: Vec<PresignedModelFileUrlResponse>,
}

#[derive(Deserialize)]
pub struct PresignedModelFileUrlResponse {
    pub rel_path: String,
    pub url: String,
}

#[derive(Deserialize)]
pub struct ModelVersionResponse {
    pub id: String,
    pub experiment: Option<ExperimentSourceResponse>,
    pub version: u32,
    pub size: u64,
    pub checksum: String,
    pub created_by: CreatedByUserResponse,
    pub created_at: String,
    pub manifest: serde_json::Value,
}

#[derive(Deserialize)]
pub struct ModelResponse {
    pub id: String,
    pub project_id: i32,
    pub name: String,
    pub description: Option<String>,
    pub created_by: CreatedByUserResponse,
    pub created_at: String,
    pub version_count: u64,
}
