use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::Digest;

use tracel_artifact::TransferObserver;
use tracel_artifact::upload::MultipartUploadSource;

use crate::{
    Model, ModelOps, ModelVersion, Models, ModelsError, VersionFile, VersionFileReader,
    VersionFileSource, VersionId,
};

#[derive(Clone)]
pub struct SourceSpec {
    pub file: VersionFile,
    pub bytes: Vec<u8>,
    pub chunk_size: usize,
    pub failure_at: Option<usize>,
    pub opens: Arc<AtomicUsize>,
    pub consumed: Arc<AtomicUsize>,
    pub opened_paths: Arc<Mutex<Vec<String>>>,
}

impl SourceSpec {
    pub fn new(path: &str, bytes: &[u8]) -> Self {
        Self {
            file: VersionFile {
                rel_path: path.to_string(),
                size_bytes: bytes.len() as u64,
                checksum: checksum(bytes),
            },
            bytes: bytes.to_vec(),
            chunk_size: usize::MAX,
            failure_at: None,
            opens: Arc::new(AtomicUsize::new(0)),
            consumed: Arc::new(AtomicUsize::new(0)),
            opened_paths: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn source(&self) -> Box<dyn VersionFileSource> {
        Box::new(TestSource(self.clone()))
    }
}

struct TestSource(SourceSpec);

impl VersionFileSource for TestSource {
    fn file(&self) -> &VersionFile {
        &self.0.file
    }

    fn open(&self, canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        self.0
            .opened_paths
            .lock()
            .unwrap()
            .push(canonical_path.to_string());
        Ok(Box::new(TestReader {
            bytes: self.0.bytes.clone(),
            chunk_size: self.0.chunk_size,
            failure_at: self.0.failure_at,
            offset: 0,
            consumed: Arc::clone(&self.0.consumed),
        }))
    }
}

struct TestReader {
    bytes: Vec<u8>,
    chunk_size: usize,
    failure_at: Option<usize>,
    offset: usize,
    consumed: Arc<AtomicUsize>,
}

impl Read for TestReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self
            .failure_at
            .is_some_and(|failure| self.offset >= failure)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "source failed mid-stream",
            ));
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }

        let before_failure = self
            .failure_at
            .map_or(usize::MAX, |failure| failure.saturating_sub(self.offset));
        let read = buffer
            .len()
            .min(self.chunk_size)
            .min(before_failure)
            .min(self.bytes.len() - self.offset);
        buffer[..read].copy_from_slice(&self.bytes[self.offset..self.offset + read]);
        self.offset += read;
        self.consumed.fetch_add(read, Ordering::SeqCst);
        Ok(read)
    }
}

/// What a publish handed the backend, so a test can see what was measured and written.
#[derive(Default)]
pub struct PublishRecord {
    pub files: Vec<VersionFile>,
    pub metadata: Option<serde_json::Value>,
    pub uploaded: Vec<String>,
}

#[derive(Clone)]
pub struct FakeOps {
    models: Vec<Model>,
    sources: Vec<SourceSpec>,
    published: Arc<Mutex<PublishRecord>>,
}

impl FakeOps {
    pub fn new(sources: Vec<SourceSpec>) -> Self {
        Self {
            models: vec![model("alpha"), model("beta")],
            sources,
            published: Arc::new(Mutex::new(PublishRecord::default())),
        }
    }

    pub fn publish_record(&self) -> Arc<Mutex<PublishRecord>> {
        Arc::clone(&self.published)
    }
}

impl ModelOps for FakeOps {
    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        let mut created = model(name);
        created.description = description.map(str::to_string);
        Ok(created)
    }

    fn publish_version(
        &self,
        model: &str,
        files: &[VersionFile],
        contents: &dyn MultipartUploadSource,
        metadata: Option<&serde_json::Value>,
        observer: &mut dyn TransferObserver,
    ) -> Result<ModelVersion, ModelsError> {
        self.get_model(model)?;
        for file in files {
            let len = contents
                .file_len(&file.rel_path)
                .map_err(ModelsError::other)?;
            if len != file.size_bytes {
                return Err(ModelsError::other(
                    "the measured size does not match the source",
                ));
            }
        }

        for file in files {
            observer.file_started(&file.rel_path, Some(file.size_bytes));
            observer.file_completed(&file.rel_path, file.size_bytes);
        }

        let mut record = self.published.lock().unwrap();
        record.files = files.to_vec();
        record.metadata = metadata.cloned();
        record.uploaded = files.iter().map(|file| file.rel_path.clone()).collect();

        Ok(version(VersionId::new("published-id")))
    }

    fn list_models(&self) -> Result<Vec<Model>, ModelsError> {
        Ok(self.models.clone())
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.models
            .iter()
            .find(|model| model.name == name)
            .cloned()
            .ok_or_else(|| ModelsError::ModelNotFound {
                name: name.to_string(),
            })
    }

    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        self.get_model(model)?;
        Ok(Vec::new())
    }

    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        self.get_model(model)?;
        Err(ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        })
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        self.get_model(model)?;
        if id.as_str() != "version-id" {
            return Err(ModelsError::VersionNotFound {
                model: model.to_string(),
                id: id.clone(),
            });
        }

        Ok(self.sources.iter().map(SourceSpec::source).collect())
    }
}

fn version(id: VersionId) -> ModelVersion {
    ModelVersion {
        id,
        version: 1,
        size_bytes: 0,
        checksum: String::new(),
        published_by: Some("publisher".to_string()),
        created_at: None,
        manifest: crate::VersionManifest { files: Vec::new() },
        metadata: serde_json::Value::Null,
    }
}

fn model(name: &str) -> Model {
    Model {
        id: format!("{name}-id"),
        name: name.to_string(),
        description: None,
        published_by: Some("publisher".to_string()),
        created_at: None,
        version_count: 0,
        latest_version: None,
    }
}

pub fn models_with_sources(sources: Vec<SourceSpec>) -> Models {
    Models::new(Arc::new(FakeOps::new(sources)))
}

pub fn checksum(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
