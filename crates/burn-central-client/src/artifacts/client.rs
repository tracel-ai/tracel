use sha2::Digest;
use std::collections::BTreeMap;

use crate::api::{ApiError, ArtifactFileSpecRequest, Client, CreateArtifactRequest};
use crate::artifacts::ArtifactInfo;
use crate::bundle::{BundleDecode, BundleEncode, InMemoryBundleReader, InMemoryBundleSources};
use crate::schemas::ExperimentPath;

#[derive(Debug, Clone, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum ArtifactKind {
    Model,
    Log,
    Other,
}

/// A scope for artifact operations within a specific experiment
#[derive(Clone)]
pub struct ExperimentArtifactClient {
    client: Client,
    exp_path: ExperimentPath,
}

impl ExperimentArtifactClient {
    pub(crate) fn new(client: Client, exp_path: ExperimentPath) -> Self {
        Self { client, exp_path }
    }

    /// Upload an artifact using the BundleEncode trait
    pub fn upload<E: BundleEncode>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<String, ArtifactError> {
        let name = name.into();
        let mut sources = InMemoryBundleSources::new();
        artifact.encode(&mut sources, settings).map_err(|e| {
            ArtifactError::Encoding(format!("Failed to encode artifact: {}", e.into()))
        })?;

        // Build file specs with size and checksum
        let mut specs = Vec::with_capacity(sources.files().len());
        for f in sources.files() {
            let (checksum, size) = sha256_and_size_from_bytes(f.source());
            specs.push(ArtifactFileSpecRequest {
                rel_path: f.dest_path().to_string(),
                size_bytes: size,
                checksum,
            });
        }

        let res = self.client.create_artifact(
            self.exp_path.owner_name(),
            self.exp_path.project_name(),
            self.exp_path.experiment_num(),
            CreateArtifactRequest {
                name: name.clone(),
                kind: kind.to_string(),
                files: specs,
            },
        )?;

        let mut url_map: BTreeMap<String, String> = BTreeMap::new();
        for f in res.files {
            url_map.insert(f.rel_path, f.url);
        }

        for f in sources.into_files() {
            let url = url_map.get(f.dest_path()).ok_or_else(|| {
                ArtifactError::Internal(format!("Missing upload URL for file {}", f.dest_path()))
            })?;

            self.client.upload_bytes_to_url(url, f.source().to_vec())?;
        }

        Ok(res.id)
    }

    /// Download an artifact and decode it using the BundleDecode trait
    pub fn download<D: BundleDecode>(
        &self,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ArtifactError> {
        let reader = self.download_raw(name.as_ref())?;
        D::decode(&reader, settings).map_err(|e| {
            ArtifactError::Decoding(format!(
                "Failed to decode artifact {}: {}",
                name.as_ref(),
                e.into()
            ))
        })
    }

    /// Download an artifact as a raw memory bundle reader
    pub fn download_raw(
        &self,
        name: impl AsRef<str>,
    ) -> Result<InMemoryBundleReader, ArtifactError> {
        let name = name.as_ref();
        let artifact = self.fetch(name)?;
        let resp = self.client.presign_artifact_download(
            self.exp_path.owner_name(),
            self.exp_path.project_name(),
            self.exp_path.experiment_num(),
            &artifact.id.to_string(),
        )?;

        let mut data = BTreeMap::new();

        for file in resp.files {
            data.insert(
                file.rel_path.clone(),
                self.client.download_bytes_from_url(&file.url)?,
            );
        }

        Ok(InMemoryBundleReader::new(data))
    }

    /// Fetch information about an artifact by name
    pub fn fetch(&self, name: impl AsRef<str>) -> Result<ArtifactInfo, ArtifactError> {
        let name = name.as_ref();
        let artifact_resp = self
            .client
            .list_artifacts_by_name(
                self.exp_path.owner_name(),
                self.exp_path.project_name(),
                self.exp_path.experiment_num(),
                name,
            )?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| ArtifactError::NotFound(name.to_owned()))?;

        Ok(artifact_resp.into())
    }
}

fn sha256_and_size_from_bytes(bytes: &[u8]) -> (String, u64) {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    (format!("{:x}", digest), bytes.len() as u64)
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Artifact not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Client(#[from] ApiError),
    #[error("Error while encoding artifact: {0}")]
    Encoding(String),
    #[error("Error while decoding artifact: {0}")]
    Decoding(String),
    #[error("Internal error: {0}")]
    Internal(String),
}
