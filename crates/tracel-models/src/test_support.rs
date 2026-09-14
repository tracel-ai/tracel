use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::TryStreamExt;
use sha2::Digest;

use tracel_artifact::upload::MultipartUploadSource;
use tracel_artifact::{ByteStream, TransferError, TransferObserver, reader_stream};
use tracel_task::{DynFuture, ThreadSpawn};

use crate::{
    Model, ModelOps, ModelVersion, Models, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionSpec,
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

    fn open<'a>(
        &'a self,
        canonical_path: &'a str,
    ) -> DynFuture<'a, Result<ByteStream, ModelsError>> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        self.0
            .opened_paths
            .lock()
            .unwrap()
            .push(canonical_path.to_string());
        let reader = TestReader {
            bytes: self.0.bytes.clone(),
            chunk_size: self.0.chunk_size,
            failure_at: self.0.failure_at,
            offset: 0,
            consumed: Arc::clone(&self.0.consumed),
        };
        Box::pin(async move {
            let body: ByteStream = Box::pin(
                reader_stream(reader).map_err(|error| TransferError::Transport(error.to_string())),
            );
            Ok(body)
        })
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

impl FakeOps {
    fn find_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.models
            .iter()
            .find(|model| model.name == name)
            .cloned()
            .ok_or_else(|| ModelsError::ModelNotFound {
                name: name.to_string(),
            })
    }
}

impl ModelOps for FakeOps {
    fn create_model<'a>(
        &'a self,
        name: &'a str,
        description: Option<&'a str>,
    ) -> DynFuture<'a, Result<Model, ModelsError>> {
        let mut created = model(name);
        created.description = description.map(str::to_string);
        Box::pin(async move { Ok(created) })
    }

    fn publish_version<'a>(
        &'a self,
        model: &'a str,
        files: &'a [VersionFile],
        contents: &'a dyn MultipartUploadSource,
        metadata: Option<&'a serde_json::Value>,
        observer: &'a mut dyn TransferObserver,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>> {
        Box::pin(async move {
            self.find_model(model)?;
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
        })
    }

    fn list_models(&self) -> DynFuture<'_, Result<Vec<Model>, ModelsError>> {
        Box::pin(async move { Ok(self.models.clone()) })
    }

    fn get_model<'a>(&'a self, name: &'a str) -> DynFuture<'a, Result<Model, ModelsError>> {
        Box::pin(async move { self.find_model(name) })
    }

    fn list_versions<'a>(
        &'a self,
        model: &'a str,
    ) -> DynFuture<'a, Result<Vec<ModelVersion>, ModelsError>> {
        Box::pin(async move {
            self.find_model(model)?;
            Ok(Vec::new())
        })
    }

    fn get_version<'a>(
        &'a self,
        model: &'a str,
        spec: VersionSpec,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>> {
        Box::pin(async move {
            self.find_model(model)?;
            Err(ModelsError::VersionNotFound {
                model: model.to_string(),
                version: spec,
            })
        })
    }

    fn fetch_version_files<'a>(
        &'a self,
        model: &'a str,
        id: &'a VersionId,
    ) -> DynFuture<'a, Result<Vec<Box<dyn VersionFileSource>>, ModelsError>> {
        Box::pin(async move {
            self.find_model(model)?;
            if id.as_str() != "version-id" {
                return Err(ModelsError::VersionNotFound {
                    model: model.to_string(),
                    version: VersionSpec::Exact(id.clone()),
                });
            }

            Ok(self.sources.iter().map(SourceSpec::source).collect())
        })
    }
}

fn version(id: VersionId) -> ModelVersion {
    ModelVersion {
        id,
        version: Some(1),
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
    models_over(FakeOps::new(sources))
}

pub fn models_over(ops: FakeOps) -> Models {
    Models::new(Arc::new(ops), Arc::new(ThreadSpawn))
}

pub fn checksum(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
