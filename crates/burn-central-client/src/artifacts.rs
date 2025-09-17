use std::collections::BTreeMap;
use std::io::Read;

use crate::api::{ArtifactFileSpecRequest, Client, ClientError, CreateArtifactRequest};
use crate::schemas::ExperimentPath;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::Digest;

#[derive(Clone, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum ArtifactKind {
    Model,
    Log,
    Other,
}

pub trait BundleSink {
    /// Add a file by streaming its bytes. Returns computed checksum + size.
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String>;

    /// Convenience: write all bytes.
    fn put_bytes(&mut self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let mut r = std::io::Cursor::new(bytes);
        self.put_file(path, &mut r)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ArtifactEncodeError {
    #[error("bundle sink error: {0}")]
    Sink(String),
    #[error("serialization error: {0}")]
    Serialize(String),
    #[error("other: {0}")]
    Other(String),
}

#[derive(thiserror::Error, Debug)]
pub enum ArtifactDecodeError {
    #[error("bundle source error: {0}")]
    Source(String),
    #[error("deserialization error: {0}")]
    Deserialize(String),
    #[error("missing required file: {0}")]
    MissingFile(String),
    #[error("other: {0}")]
    Other(String),
}

pub trait BundleSource {
    /// Open the given path for streaming read. Must validate existence.
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String>;

    /// Optionally list available files (used by generic decoders; can be best-effort).
    fn list(&self) -> Result<Vec<String>, String>;
}

pub trait ArtifactEncode {
    type Settings: Default + Serialize + DeserializeOwned;
    type Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;
    fn encode<O: BundleSink>(
        self,
        sink: &mut O,
        settings: &Self::Settings,
    ) -> Result<(), Self::Error>;
}

pub trait ArtifactDecode: Sized {
    type Settings: Default + Serialize + DeserializeOwned;
    type Error: Into<Box<dyn std::error::Error + Send + Sync + 'static>>;
    fn decode<I: BundleSource>(source: &I, settings: &Self::Settings) -> Result<Self, Self::Error>;
}

impl BundleSink for ArtifactSources {
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String> {
        let mut buf = Vec::new();
        reader
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read from source: {}", e))?;
        *self = self.clone().add_bytes(buf, path);
        Ok(())
    }
}

impl BundleSource for MemoryArtifactReader {
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String> {
        let rel = normalize_artifact_path(path);
        let bytes = self
            .files
            .get(&rel)
            .ok_or_else(|| format!("File not found in artifact: {}", rel))?;
        Ok(Box::new(std::io::Cursor::new(bytes.clone())))
    }

    fn list(&self) -> Result<Vec<String>, String> {
        Ok(self.files.keys().cloned().collect())
    }
}

#[derive(Default, Clone)]
pub struct ArtifactSources {
    files: Vec<PendingFile>,
}

impl ArtifactSources {
    /// Add raw bytes as a file within the artifact at `dest_path`.
    pub fn add_bytes(mut self, bytes: Vec<u8>, dest_path: impl AsRef<str>) -> Self {
        self.files.push(PendingFile {
            dest_path: normalize_artifact_path(dest_path.as_ref()),
            source: bytes,
        });
        self
    }

    fn files(&self) -> &Vec<PendingFile> {
        &self.files
    }

    fn into_files(self) -> Vec<PendingFile> {
        self.files
    }
}

#[derive(Clone)]
pub struct ArtifactScope {
    client: Client,
    exp_path: ExperimentPath,
}

impl ArtifactScope {
    pub(crate) fn new(client: Client, exp_path: ExperimentPath) -> Self {
        Self { client, exp_path }
    }

    pub fn upload<E: ArtifactEncode>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<String, ArtifactError> {
        let name = name.into();
        let mut sources = ArtifactSources::default();
        artifact.encode(&mut sources, settings).map_err(|e| {
            ArtifactError::Encoding(format!("Failed to encode artifact: {}", e.into()))
        })?;

        // Build file specs with size and checksum
        let mut specs = Vec::with_capacity(sources.files().len());
        for f in sources.files() {
            let (checksum, size) = sha256_and_size_from_bytes(&f.source);
            specs.push(ArtifactFileSpecRequest {
                rel_path: f.dest_path.clone(),
                size_bytes: size,
                checksum,
            });
        }

        // 1) Ask backend to create artifact and return presigned URLs
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

        // 2) Upload all files
        for f in sources.into_files() {
            let url = url_map.get(&f.dest_path).ok_or_else(|| {
                ArtifactError::Internal(format!("Missing upload URL for file {}", f.dest_path))
            })?;

            self.client.upload_bytes_to_url(url, f.source)?;
        }

        Ok(res.id)
    }

    /// Create a dynamic reader for an existing artifact (by id) bound to this scope.
    pub fn download<D: ArtifactDecode>(
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

    /// Create a dynamic reader for an existing artifact (by id) bound to this scope.
    pub fn download_raw(
        &self,
        name: impl AsRef<str>,
    ) -> Result<MemoryArtifactReader, ArtifactError> {
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

        let resp = self.client.presign_artifact_download(
            self.exp_path.owner_name(),
            self.exp_path.project_name(),
            self.exp_path.experiment_num(),
            &artifact_resp.id,
        )?;

        let mut data = BTreeMap::new();

        for file in resp.files {
            data.insert(
                file.rel_path.clone(),
                self.client.download_bytes_from_url(&file.url)?,
            );
        }

        Ok(MemoryArtifactReader::new(data))
    }
}

#[derive(Clone)]
struct PendingFile {
    dest_path: String, // path within the artifact (use forward slashes)
    source: Vec<u8>,
}

fn normalize_artifact_path<S: AsRef<str>>(s: S) -> String {
    s.as_ref()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

fn sha256_and_size_from_bytes(bytes: &[u8]) -> (String, u64) {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    (format!("{:x}", digest), bytes.len() as u64)
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("Error while encoding artifact: {0}")]
    Encoding(String),
    #[error("Error while decoding artifact: {0}")]
    Decoding(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// In-memory reader for synthetic or cached artifacts.
pub struct MemoryArtifactReader {
    files: BTreeMap<String, Vec<u8>>, // rel_path -> bytes
}

impl MemoryArtifactReader {
    pub fn new(files: BTreeMap<String, Vec<u8>>) -> Self {
        Self { files }
    }
}
