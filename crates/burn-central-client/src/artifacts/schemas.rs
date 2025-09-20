use crate::{api::ArtifactResponse, artifacts::ArtifactKind, models::ExperimentSource};

#[derive(Debug, Clone)]
pub struct ArtifactInfo {
    pub id: uuid::Uuid,
    pub created_at: String,
    pub name: String,
    pub kind: ArtifactKind,
    pub bucket_id: String,
    pub experiment: ExperimentSource,
    pub manifest: serde_json::Value,
}

impl From<ArtifactResponse> for ArtifactInfo {
    fn from(value: ArtifactResponse) -> Self {
        ArtifactInfo {
            id: value.id.parse().unwrap_or(uuid::Uuid::nil()),
            created_at: value.created_at,
            name: value.name,
            kind: value.kind.parse().unwrap_or(ArtifactKind::Other),
            bucket_id: value.bucket_id,
            experiment: ExperimentSource {
                experiment_id: value.experiment.id,
                experiment_num: value.experiment.experiment_num,
            },
            manifest: value.manifest,
        }
    }
}
