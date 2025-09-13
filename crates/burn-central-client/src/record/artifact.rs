use crate::api::{ArtifactFileSpecRequest, ArtifactResponse, Client, CreateArtifactRequest};
use crate::schemas::ExperimentPath;
use burn::prelude::Backend;
use burn::record::{FullPrecisionSettings, Recorder, RecorderError};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::Digest;
use strum::Display;

#[derive(Clone, Display)]
#[strum(serialize_all = "snake_case")]
pub enum ArtifactKind {
    Model,
    Log,
    Other,
}

#[derive(Clone)]
pub struct ArtifactRecordArgs {
    pub experiment_path: ExperimentPath,
    pub name: String,
    pub kind: ArtifactKind,
}

pub enum ArtifactQueryArgs {
    ByName(String),
    ById(String),
}

pub struct ArtifactLoadArgs {
    pub experiment_path: ExperimentPath,
    pub query: ArtifactQueryArgs,
}

/// A recorder that saves and loads single-file artifacts to/from a remote server using the [BurnCentralClient](crate::api::Client).
/// Artifacts are serialized using `rmp_serde`.
#[derive(Clone, Debug)]
pub struct ArtifactRecorder {
    client: Client,
}

impl ArtifactRecorder {
    pub fn new(client: Client) -> Self {
        ArtifactRecorder { client }
    }

    pub fn query_artifact(
        &self,
        experiment_path: &ExperimentPath,
        query: &ArtifactQueryArgs,
    ) -> Result<ArtifactResponse, RecorderError> {
        match query {
            ArtifactQueryArgs::ById(artifact_id) => {
                let _artifact = self
                    .client
                    .get_artifact(
                        experiment_path.owner_name(),
                        experiment_path.project_name(),
                        experiment_path.experiment_num(),
                        artifact_id,
                    )
                    .map_err(|e| {
                        RecorderError::Unknown(format!("Failed to get artifact by ID: {e}"))
                    })?;
                Ok(_artifact)
            }
            ArtifactQueryArgs::ByName(artifact_name) => {
                let _artifact = self
                    .client
                    .list_artifacts_by_name(
                        experiment_path.owner_name(),
                        experiment_path.project_name(),
                        experiment_path.experiment_num(),
                        artifact_name,
                    )
                    .map_err(|e| {
                        RecorderError::Unknown(format!("Failed to get artifact by name: {e}"))
                    })?
                    .items
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        RecorderError::Unknown(format!(
                            "No artifact found with name: {}",
                            artifact_name
                        ))
                    })?;
                Ok(_artifact)
            }
        }
    }
}

impl Default for ArtifactRecorder {
    fn default() -> Self {
        unimplemented!("Default for ArtifactRecorder is not implemented, use new() instead");
    }
}

impl<B: Backend> Recorder<B> for ArtifactRecorder {
    type Settings = FullPrecisionSettings;
    type RecordArgs = ArtifactRecordArgs;
    type RecordOutput = ();
    type LoadArgs = ArtifactLoadArgs;

    fn save_item<I: Serialize>(
        &self,
        item: I,
        args: Self::RecordArgs,
    ) -> Result<Self::RecordOutput, RecorderError> {
        let serialized_bytes =
            rmp_serde::encode::to_vec_named(&item).expect("Should be able to serialize.");

        tracing::debug!(
            "Saving artifact '{}' of kind '{}' for experiment {}",
            args.name,
            args.kind,
            args.experiment_path
        );

        let size = serialized_bytes.len();
        let checksum = sha2::Sha256::new_with_prefix(&serialized_bytes).finalize();

        let req = CreateArtifactRequest {
            name: args.name.clone(),
            kind: args.kind.to_string(),
            files: vec![ArtifactFileSpecRequest {
                rel_path: args.name.clone(),
                size_bytes: size as u64,
                checksum: format!("{:x}", checksum),
            }],
        };

        let mut response = self
            .client
            .create_artifact(
                args.experiment_path.owner_name(),
                args.experiment_path.project_name(),
                args.experiment_path.experiment_num(),
                req,
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to get upload URL: {e}")))?;

        let upload_url = response
            .files
            .pop()
            .ok_or_else(|| {
                RecorderError::Unknown("No files returned from artifact creation".to_string())
            })?
            .url;

        self.client
            .upload_bytes_to_url(&upload_url, serialized_bytes)
            .map_err(|e| RecorderError::Unknown(format!("Failed to upload item: {e}")))?;

        Ok(())
    }

    fn load_item<I>(&self, args: &mut Self::LoadArgs) -> Result<I, RecorderError>
    where
        I: DeserializeOwned,
    {
        let artifact = self.query_artifact(&args.experiment_path, &args.query)?;
        let download_url = self
            .client
            .presign_artifact_download(
                args.experiment_path.owner_name(),
                args.experiment_path.project_name(),
                args.experiment_path.experiment_num(),
                &artifact.id,
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to get download URL: {e}")))?
            .files
            .pop()
            .ok_or_else(|| {
                RecorderError::Unknown("No files returned from artifact download".to_string())
            })?
            .url;

        let bytes = self
            .client
            .download_bytes_from_url(&download_url)
            .map_err(|e| RecorderError::Unknown(format!("Failed to download item: {e}")))?;

        rmp_serde::decode::from_slice(&bytes).map_err(|e| {
            RecorderError::DeserializeError(format!("Failed to deserialize item: {e}"))
        })
    }
}
