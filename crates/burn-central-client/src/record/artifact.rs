use crate::api::Client;
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
    Checkpoint,
}

#[derive(Clone)]
pub struct ArtifactRecordArgs {
    pub experiment_path: ExperimentPath,
    pub name: String,
    pub kind: ArtifactKind,
}

pub struct ArtifactLoadArgs {
    pub experiment_path: ExperimentPath,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct ArtifactRecorder {
    client: Client,
}

impl ArtifactRecorder {
    pub fn new(client: Client) -> Self {
        ArtifactRecorder { client }
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

        let mut checksum = sha2::Sha256::new();
        checksum.update(&serialized_bytes);

        let upload_url = self
            .client
            .request_artifact_save_url(
                args.experiment_path.owner_name(),
                args.experiment_path.project_name(),
                args.experiment_path.experiment_num(),
                &args.name,
                serialized_bytes.len(),
                &format!("{:x}", checksum.finalize()),
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to get upload URL: {e}")))?;

        self.client
            .upload_bytes_to_url(&upload_url, serialized_bytes)
            .map_err(|e| RecorderError::Unknown(format!("Failed to upload item: {e}")))?;

        Ok(())
    }

    fn load_item<I>(&self, args: &mut Self::LoadArgs) -> Result<I, RecorderError>
    where
        I: DeserializeOwned,
    {
        let download_url = self
            .client
            .request_artifact_load_url(
                args.experiment_path.owner_name(),
                args.experiment_path.project_name(),
                args.experiment_path.experiment_num(),
                &args.name,
            )
            .map_err(|e| RecorderError::Unknown(format!("Failed to get download URL: {e}")))?;

        let bytes = self
            .client
            .download_bytes_from_url(&download_url)
            .map_err(|e| RecorderError::Unknown(format!("Failed to download item: {e}")))?;

        rmp_serde::decode::from_slice(&bytes).map_err(|e| {
            RecorderError::DeserializeError(format!("Failed to deserialize item: {e}"))
        })
    }
}
