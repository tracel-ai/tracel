//! Fluent multi-file artifact builder for experiments.
//! Example:
//!
//! let artifact_id = burn_central
//!     .artifacts("owner", "project", 42)?
//!     .builder("mnist-v1", ArtifactKind::Other)
//!     .add_file("./README.md", Some("docs/readme.md"))
//!     .add_dir("./data", "data")?
//!     .add_bytes(vec![1,2,3], "meta/raw.bin")
//!     .upload()?;
//!
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::api::{ArtifactFileSpecRequest, Client, ClientError, CreateArtifactRequest};
use crate::client::BurnCentralError;
use crate::record::ArtifactKind;
use crate::schemas::ExperimentPath;
use sha2::Digest;

pub trait IntoArtifactSources {
    fn into_artifact_sources(self) -> ArtifactSources;
}

impl IntoArtifactSources for ArtifactSources {
    fn into_artifact_sources(self) -> ArtifactSources {
        self
    }
}

#[derive(Default, Clone)]
pub struct ArtifactSources {
    files: Vec<PendingFile>,
}

impl ArtifactSources {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Add a file from disk. If `dest_path` is None, uses the filename.
    pub fn add_file(mut self, src_path: impl AsRef<Path>, dest_path: Option<&str>) -> Self {
        let dest = match dest_path {
            Some(p) => normalize_artifact_path(p),
            None => normalize_artifact_path(
                src_path
                    .as_ref()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            ),
        };
        self.files.push(PendingFile {
            dest_path: dest,
            source: Source::Path(src_path.as_ref().to_path_buf()),
        });
        self
    }

    /// Add raw bytes as a file within the artifact at `dest_path`.
    pub fn add_bytes(mut self, bytes: Vec<u8>, dest_path: impl AsRef<str>) -> Self {
        self.files.push(PendingFile {
            dest_path: normalize_artifact_path(dest_path.as_ref()),
            source: Source::Bytes(bytes),
        });
        self
    }

    /// Recursively add a directory. Files are added under `dest_root` inside the artifact.
    /// The destination will mirror the relative structure under `dir`.
    pub fn add_dir(
        mut self,
        dir: impl AsRef<Path>,
        dest_root: impl AsRef<str>,
    ) -> Result<Self, BurnCentralError> {
        let dir = dir.as_ref();
        if !dir.is_dir() {
            return Err(BurnCentralError::Internal(format!(
                "Not a directory: {}",
                dir.display()
            )));
        }

        let root = dir.canonicalize().map_err(|e| {
            BurnCentralError::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to canonicalize directory {}: {}", dir.display(), e),
            ))
        })?;

        let dest_root = normalize_artifact_path(dest_root.as_ref());
        let mut stack = vec![root.clone()];
        while let Some(current) = stack.pop() {
            let entries = fs::read_dir(&current)?;
            for entry in entries {
                let entry = entry?;
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.is_file() {
                    let rel = p.strip_prefix(&root).unwrap_or(&p);
                    let rel_norm = normalize_artifact_path(rel.to_string_lossy());
                    let dest_path = if dest_root.is_empty() {
                        rel_norm
                    } else {
                        format!("{}/{}", dest_root, rel_norm)
                    };
                    self.files.push(PendingFile {
                        dest_path,
                        source: Source::Path(p),
                    });
                }
            }
        }

        Ok(self)
    }

    pub fn add_sources(mut self, other: impl IntoArtifactSources) -> Self {
        self.files.extend(other.into_artifact_sources().files);
        self
    }

    fn files(&self) -> &Vec<PendingFile> {
        &self.files
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

    /// Start a new artifact builder for this scope.
    pub fn builder(&self, name: impl Into<String>, kind: ArtifactKind) -> ArtifactBuilder {
        ArtifactBuilder::new(
            self.client.clone(),
            self.exp_path.clone(),
            name.into(),
            kind,
        )
    }

    /// Create a dynamic reader for an existing artifact (by id) bound to this scope.
    pub fn fetch(
        &self,
        name: impl Into<String>,
    ) -> Result<Box<dyn ArtifactReader>, ArtifactReadError> {
        let name = name.into();
        let artifact_resp = self
            .client
            .list_artifacts_by_name(
                self.exp_path.owner_name(),
                self.exp_path.project_name(),
                self.exp_path.experiment_num(),
                &name,
            )?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| ArtifactReadError::NotFound(name))?;

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

        Ok(Box::new(MemoryArtifactReader::new(data)))
    }

    /// Create a dynamic reader for an existing artifact by name (first match).
    pub fn reader_by_name(&self, name: &str) -> Result<Box<dyn ArtifactReader>, ArtifactReadError> {
        // Reuse existing list_artifacts_by_name endpoint
        let list = self.client.list_artifacts_by_name(
            self.exp_path.owner_name(),
            self.exp_path.project_name(),
            self.exp_path.experiment_num(),
            name,
        )?;
        let artifact = list
            .items
            .into_iter()
            .next()
            .ok_or_else(|| ArtifactReadError::NotFound(name.to_string()))?;
        self.fetch(&artifact.id)
    }
}

#[derive(Clone)]
pub struct ArtifactBuilder {
    client: Client,
    exp_path: ExperimentPath,
    name: String,
    kind: ArtifactKind,
    sources: ArtifactSources,
}

#[derive(Clone)]
enum Source {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
struct PendingFile {
    dest_path: String, // path within the artifact (use forward slashes)
    source: Source,
}

impl ArtifactBuilder {
    fn new(client: Client, exp_path: ExperimentPath, name: String, kind: ArtifactKind) -> Self {
        Self {
            client,
            exp_path,
            name,
            kind,
            sources: ArtifactSources::new(),
        }
    }

    fn validate(&self) -> Result<(), ArtifactBuilderError> {
        if self.name.trim().is_empty() {
            return Err(ArtifactBuilderError::EmptyName);
        }
        if self.sources.files().is_empty() {
            return Err(ArtifactBuilderError::NoFiles);
        }
        let mut seen = HashSet::new();
        for f in &self.sources.files {
            if !seen.insert(f.dest_path.clone()) {
                return Err(ArtifactBuilderError::DuplicateDestPath(f.dest_path.clone()));
            }
        }
        Ok(())
    }

    pub fn add_sources(mut self, other: impl IntoArtifactSources) -> Self {
        self.sources = self.sources.add_sources(other);
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactBuilderError {
    #[error("Artifact name cannot be empty")]
    EmptyName,
    #[error("Artifact must contain at least one file")]
    NoFiles,
    #[error("Duplicate dest path in artifact: {0}")]
    DuplicateDestPath(String),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Failed to read file")]
    Io(#[from] std::io::Error),
    #[error("Failed to upload artifact")]
    UploadError(#[from] ClientError),
}

impl ArtifactBuilder {
    /// Create the artifact on the server, upload files, and return the artifact id.
    pub fn upload(self) -> Result<String, ArtifactBuilderError> {
        self.validate()?;

        // Build file specs with size and checksum
        let mut specs = Vec::with_capacity(self.sources.files().len());
        for f in self.sources.files() {
            match &f.source {
                Source::Path(p) => {
                    let (checksum, size) = sha256_and_size_from_path(p)?;
                    specs.push(ArtifactFileSpecRequest {
                        rel_path: f.dest_path.clone(),
                        size_bytes: size,
                        checksum,
                    });
                }
                Source::Bytes(bytes) => {
                    let (checksum, size) = sha256_and_size_from_bytes(bytes);
                    specs.push(ArtifactFileSpecRequest {
                        rel_path: f.dest_path.clone(),
                        size_bytes: size,
                        checksum,
                    });
                }
            }
        }

        // 1) Ask backend to create artifact and return presigned URLs
        let res = self.client.create_artifact(
            self.exp_path.owner_name(),
            self.exp_path.project_name(),
            self.exp_path.experiment_num(),
            CreateArtifactRequest {
                name: self.name.clone(),
                kind: self.kind.to_string(),
                files: specs,
            },
        )?;

        let mut url_map: BTreeMap<String, String> = BTreeMap::new();
        for f in res.files {
            url_map.insert(f.rel_path, f.url);
        }

        // 2) Upload all files
        for f in self.sources.files() {
            let url = url_map.get(&f.dest_path).ok_or_else(|| {
                ArtifactBuilderError::Internal(format!(
                    "No upload URL for artifact file: {}",
                    f.dest_path
                ))
            })?;

            let bytes = match &f.source {
                Source::Path(p) => fs::read(p).map_err(|e| {
                    std::io::Error::new(
                        e.kind(),
                        format!("Failed to read file {}: {}", p.display(), e),
                    )
                })?,
                Source::Bytes(b) => b.clone(),
            };

            self.client.upload_bytes_to_url(url, bytes)?;
        }

        Ok(res.id)
    }
}

fn normalize_artifact_path<S: AsRef<str>>(s: S) -> String {
    s.as_ref()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

fn sha256_and_size_from_path(path: &Path) -> Result<(String, u64), std::io::Error> {
    let mut file = fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }

    let digest = hasher.finalize();
    Ok((format!("{:x}", digest), total))
}

fn sha256_and_size_from_bytes(bytes: &[u8]) -> (String, u64) {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    (format!("{:x}", digest), bytes.len() as u64)
}

// BurnCentral convenience entrypoints are implemented in client.rs to access private fields.

// ============================
// Dynamic Artifact Reader API
// ============================

#[derive(Debug, thiserror::Error)]
pub enum ArtifactReadError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("Internal error: {0}")]
    Internal(String),
}

/// A dynamic reader for multi-file artifacts.
pub trait ArtifactReader: Send + Sync {
    /// List all file paths inside the artifact (normalized with forward slashes).
    fn list_paths(&self) -> Result<Vec<String>, ArtifactReadError>;
    /// Read the bytes of a specific file by its relative path.
    fn read(&self, rel_path: &str) -> Result<Vec<u8>, ArtifactReadError>;

    /// Convenience: check if a given file exists in the artifact.
    fn exists(&self, rel_path: &str) -> Result<bool, ArtifactReadError> {
        let rel = normalize_artifact_path(rel_path);
        Ok(self.list_paths()?.into_iter().any(|p| p == rel))
    }
}

pub trait ArtifactDecoder: Sized {
    fn decode(reader: &dyn ArtifactReader) -> Result<Self, ArtifactReadError>;
}

impl ArtifactDecoder for () {
    fn decode(_reader: &dyn ArtifactReader) -> Result<Self, ArtifactReadError> {
        Ok(())
    }
}

impl ArtifactDecoder for serde_json::Value {
    fn decode(reader: &dyn ArtifactReader) -> Result<Self, ArtifactReadError> {
        let bytes = reader.read("config.json")?;
        let v = serde_json::from_slice(&bytes).map_err(|e| {
            ArtifactReadError::Internal(format!("Failed to deserialize config.json: {e}"))
        })?;
        Ok(v)
    }
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

impl ArtifactReader for MemoryArtifactReader {
    fn list_paths(&self) -> Result<Vec<String>, ArtifactReadError> {
        Ok(self.files.keys().cloned().collect())
    }

    fn read(&self, rel_path: &str) -> Result<Vec<u8>, ArtifactReadError> {
        let rel = normalize_artifact_path(rel_path);
        self.files
            .get(&rel)
            .cloned()
            .ok_or_else(|| ArtifactReadError::NotFound(rel))
    }
}
