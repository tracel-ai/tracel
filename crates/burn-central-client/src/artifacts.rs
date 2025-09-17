use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::api::{ArtifactFileSpecRequest, Client, ClientError, CreateArtifactRequest};
use crate::client::BurnCentralError;
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

    pub fn upload<A: ArtifactEncode>(
        &self,
        name: impl Into<String>,
        kind: ArtifactKind,
        artifact: A,
        settings: &A::Settings,
    ) -> Result<String, String> {
        let name = name.into();
        let mut sources = ArtifactSources::new();
        artifact
            .encode(&mut sources, settings)
            .map_err(|e| format!("Failed to encode artifact: {}", e.into()))?;
        // self.validate()?;

        // Build file specs with size and checksum
        let mut specs = Vec::with_capacity(sources.files().len());
        for f in sources.files() {
            match &f.source {
                Source::Path(p) => {
                    let (checksum, size) = sha256_and_size_from_path(p)
                        .map_err(|e| format!("Failed to read file {}: {}", p.display(), e))?;
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
        let res = self
            .client
            .create_artifact(
                self.exp_path.owner_name(),
                self.exp_path.project_name(),
                self.exp_path.experiment_num(),
                CreateArtifactRequest {
                    name: name.clone(),
                    kind: kind.to_string(),
                    files: specs,
                },
            )
            .map_err(|e| format!("Failed to create artifact: {}", e))?;

        let mut url_map: BTreeMap<String, String> = BTreeMap::new();
        for f in res.files {
            url_map.insert(f.rel_path, f.url);
        }

        // 2) Upload all files
        for f in sources.files() {
            let url = url_map.get(&f.dest_path).ok_or_else(|| {
                format!(
                    "Internal error: missing upload URL for file {}",
                    f.dest_path
                )
            })?;

            let bytes = match &f.source {
                Source::Path(p) => fs::read(p)
                    .map_err(|e| format!("Failed to read file {}: {}", p.display(), e))?,
                Source::Bytes(b) => b.clone(),
            };

            self.client.upload_bytes_to_url(url, bytes).map_err(|e| {
                format!(
                    "Failed to upload file {} to URL: {}",
                    f.dest_path,
                    e
                )
            })?;
        }

        Ok(res.id)
    }

    /// Create a dynamic reader for an existing artifact (by id) bound to this scope.
    pub fn fetch(&self, name: impl AsRef<str>) -> Result<MemoryArtifactReader, ArtifactReadError> {
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
            .ok_or_else(|| ArtifactReadError::NotFound(name.to_owned()))?;

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
enum Source {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
struct PendingFile {
    dest_path: String, // path within the artifact (use forward slashes)
    source: Source,
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

/// In-memory reader for synthetic or cached artifacts.
pub struct MemoryArtifactReader {
    files: BTreeMap<String, Vec<u8>>, // rel_path -> bytes
}

impl MemoryArtifactReader {
    pub fn new(files: BTreeMap<String, Vec<u8>>) -> Self {
        Self { files }
    }
}
