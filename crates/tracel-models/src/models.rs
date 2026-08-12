use std::collections::HashSet;
use std::fmt;
use std::io::Read;
use std::sync::{Arc, Mutex};

use tracel_artifact::bundle::{BundleDecode, BundleSink, BundleSource, FsBundle};
use tracel_artifact::download::{
    ArtifactDownloadFile, DownloadError, DownloadObserver,
    download_artifacts_to_sink_with_client_and_observer,
};
use tracel_artifact::{FileTransferClient, TransferError, normalize_checksum};

use crate::{
    Model, ModelOps, ModelVersion, ModelsError, Page, VersionFileReader, VersionFileSource,
    VersionId,
};

/// Backend-independent model operations and verified transfer orchestration.
#[derive(Clone)]
pub struct Models {
    ops: Arc<dyn ModelOps>,
}

impl Models {
    /// Creates a capability over backend primitives that are already scoped to their location.
    pub fn new(ops: Arc<dyn ModelOps>) -> Self {
        Self { ops }
    }

    /// Lists models in this capability's scope.
    pub fn list(&self) -> Result<Page<Model>, ModelsError> {
        self.ops.list_models()
    }

    /// Fetches one model by name.
    pub fn get(&self, name: &str) -> Result<Model, ModelsError> {
        self.ops.get_model(name)
    }

    /// Lists published versions of a model.
    pub fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        self.ops.list_versions(model)
    }

    /// Fetches one version using its opaque identity.
    pub fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        self.ops.get_version(model, id)
    }

    /// Downloads and verifies a version before copying it into a caller-owned bundle sink.
    ///
    /// Progress callbacks run synchronously while backend bytes are staged. The destination is
    /// untouched unless every file passes path, size, and checksum verification.
    pub fn download<S, O>(
        &self,
        model: &str,
        id: &VersionId,
        destination: &mut S,
        observer: &mut O,
    ) -> Result<(), ModelsError>
    where
        S: BundleSink,
        O: DownloadObserver,
    {
        let staged = self.stage(model, id, observer)?;
        staged.copy_to(destination)
    }

    /// Downloads, verifies, and decodes a model version using `settings`.
    pub fn load<D: BundleDecode>(
        &self,
        model: &str,
        id: &VersionId,
        settings: &D::Settings,
    ) -> Result<D, ModelsError> {
        let staged = self.stage(model, id, &mut ())?;
        D::decode(&staged, settings).map_err(|error| {
            let error: Box<dyn std::error::Error + Send + Sync> = error.into();
            ModelsError::Decode(error.to_string())
        })
    }

    fn stage<O: DownloadObserver>(
        &self,
        model: &str,
        id: &VersionId,
        observer: &mut O,
    ) -> Result<StagedVersion, ModelsError> {
        let sources: Arc<[VersionFileSource]> = self.ops.fetch_version_files(model, id)?.into();
        let paths = validated_source_paths(&sources)?;
        let mut files = Vec::with_capacity(sources.len());

        for (source, path) in sources.iter().zip(paths) {
            files.push(stage_source(source, path, observer)?);
        }

        StagedVersion { sources, files }.persist_cache()
    }
}

impl fmt::Debug for Models {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Models").finish_non_exhaustive()
    }
}

struct StagedFile {
    bundle: FsBundle,
    path: String,
    cache_hit: bool,
}

struct StagedVersion {
    sources: Arc<[VersionFileSource]>,
    files: Vec<StagedFile>,
}

impl StagedVersion {
    fn persist_cache(self) -> Result<Self, ModelsError> {
        for (source, file) in self.sources.iter().zip(&self.files) {
            if file.cache_hit || !source.has_cache() {
                continue;
            }

            let mut reader = file.bundle.open(&file.path).map_err(ModelsError::Staging)?;
            source.store_verified(&file.path, reader.as_mut())?;
        }
        Ok(self)
    }

    fn copy_to<S: BundleSink>(&self, destination: &mut S) -> Result<(), ModelsError> {
        for file in &self.files {
            let mut reader = file.bundle.open(&file.path).map_err(ModelsError::Staging)?;
            destination
                .put_file(&file.path, &mut reader)
                .map_err(ModelsError::Destination)?;
        }
        Ok(())
    }
}

impl BundleSource for StagedVersion {
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String> {
        let canonical = canonical_version_path(path).map_err(|error| error.to_string())?;
        let file = self
            .files
            .iter()
            .find(|file| file.path == canonical)
            .ok_or_else(|| format!("Bundle path not found: {canonical}"))?;
        file.bundle.open(&file.path)
    }

    fn list(&self) -> Result<Vec<String>, String> {
        Ok(self.files.iter().map(|file| file.path.clone()).collect())
    }
}

fn stage_source<O: DownloadObserver>(
    source: &VersionFileSource,
    path: String,
    observer: &mut O,
) -> Result<StagedFile, ModelsError> {
    if let Some(reader) = source.open_cached(&path)? {
        let mut buffered = BufferedObserver::default();
        match stage_reader(source, &path, reader, ReaderOrigin::Cache, &mut buffered) {
            Ok(bundle) => {
                buffered.replay(observer);
                return Ok(StagedFile {
                    bundle,
                    path,
                    cache_hit: true,
                });
            }
            Err(error) if cached_content_failed(&error) => {
                source.invalidate_cached(&path)?;
            }
            Err(error) => return Err(error),
        }
    }

    let reader = source.open_authoritative()?;
    let bundle = stage_reader(source, &path, reader, ReaderOrigin::Authoritative, observer)?;
    Ok(StagedFile {
        bundle,
        path,
        cache_hit: false,
    })
}

fn cached_content_failed(error: &ModelsError) -> bool {
    matches!(
        error,
        ModelsError::Cache(_)
            | ModelsError::Verification(
                DownloadError::SizeMismatch { .. } | DownloadError::ChecksumMismatch { .. }
            )
    )
}

fn stage_reader<O: DownloadObserver>(
    source: &VersionFileSource,
    path: &str,
    reader: VersionFileReader,
    origin: ReaderOrigin,
    observer: &mut O,
) -> Result<FsBundle, ModelsError> {
    let mut bundle = FsBundle::temp().map_err(|error| ModelsError::Staging(error.to_string()))?;
    let read_failure = Arc::new(Mutex::new(None));
    let client = ReaderTransferClient::new(TrackingReader {
        inner: reader,
        origin,
        failure: Arc::clone(&read_failure),
    });
    let file = ArtifactDownloadFile {
        rel_path: path.to_string(),
        url: "model-source".to_string(),
        size_bytes: Some(source.file().size_bytes),
        checksum: Some(source.file().checksum.clone()),
    };

    let result = download_artifacts_to_sink_with_client_and_observer(
        &client,
        &mut bundle,
        &[file],
        observer,
    );

    match result {
        Ok(()) => Ok(bundle),
        Err(error) => {
            let read_failure = read_failure
                .lock()
                .map_err(|_| ModelsError::Staging("model reader state failed".to_string()))?
                .take();
            Err(read_failure.unwrap_or_else(|| map_download_error(error)))
        }
    }
}

fn map_download_error(error: DownloadError) -> ModelsError {
    match error {
        DownloadError::Transfer { .. } => ModelsError::Transport(error.to_string()),
        DownloadError::TargetError(message) => ModelsError::Staging(message),
        error => ModelsError::Verification(error),
    }
}

fn validated_source_paths(sources: &[VersionFileSource]) -> Result<Vec<String>, ModelsError> {
    let mut seen = HashSet::with_capacity(sources.len());
    let mut paths: Vec<String> = Vec::with_capacity(sources.len());

    for source in sources {
        let checksum = normalize_checksum(&source.file().checksum)
            .map_err(|error| ModelsError::Verification(DownloadError::InvalidChecksum(error)))?;
        if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ModelsError::Verification(DownloadError::InvalidChecksum(
                format!(
                    "expected a 64-digit SHA-256 checksum, got '{}'",
                    source.file().checksum
                ),
            )));
        }
        let path = canonical_version_path(&source.file().rel_path)?;
        if !seen.insert(path.clone()) {
            return Err(invalid_path(format!(
                "duplicate relative model path: {path}"
            )));
        }
        if paths.iter().any(|existing| paths_conflict(existing, &path)) {
            return Err(invalid_path(format!(
                "model path conflicts with another file or directory: {path}"
            )));
        }
        paths.push(path);
    }

    Ok(paths)
}

fn paths_conflict(left: &str, right: &str) -> bool {
    left.strip_prefix(right)
        .is_some_and(|rest| rest.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn canonical_version_path(path: &str) -> Result<String, ModelsError> {
    let normalized = path.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(invalid_path(format!("model path must be relative: {path}")));
    }

    let mut segments = Vec::new();
    for segment in normalized.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                return Err(invalid_path(format!(
                    "model path escapes its bundle: {path}"
                )));
            }
            segment => {
                validate_portable_segment(segment, path)?;
                segments.push(segment);
            }
        }
    }

    if segments.is_empty() {
        return Err(invalid_path("empty relative model path".to_string()));
    }

    Ok(segments.join("/"))
}

fn validate_portable_segment(segment: &str, path: &str) -> Result<(), ModelsError> {
    if segment.ends_with('.')
        || segment.ends_with(' ')
        || segment
            .chars()
            .any(|character| character.is_ascii_control() || r#"<>:"|?*"#.contains(character))
    {
        return Err(invalid_path(format!("model path is not portable: {path}")));
    }

    let device_name = segment.split('.').next().unwrap_or(segment);
    let device_name = device_name.to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || device_name.strip_prefix("COM").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        })
        || device_name.strip_prefix("LPT").is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        });
    if reserved {
        return Err(invalid_path(format!(
            "model path uses a reserved component: {path}"
        )));
    }

    Ok(())
}

fn invalid_path(message: String) -> ModelsError {
    ModelsError::Verification(DownloadError::InvalidPath(message))
}

#[derive(Clone, Copy)]
enum ReaderOrigin {
    Authoritative,
    Cache,
}

struct TrackingReader {
    inner: VersionFileReader,
    origin: ReaderOrigin,
    failure: Arc<Mutex<Option<ModelsError>>>,
}

impl Read for TrackingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(read) => Ok(read),
            Err(error) => {
                let failure = match self.origin {
                    ReaderOrigin::Authoritative => ModelsError::Transport(error.to_string()),
                    ReaderOrigin::Cache => ModelsError::Cache(error.to_string()),
                };
                if let Ok(mut recorded) = self.failure.lock() {
                    *recorded = Some(failure);
                }
                Err(error)
            }
        }
    }
}

#[derive(Clone)]
struct ReaderTransferClient {
    reader: Arc<Mutex<Option<TrackingReader>>>,
}

impl ReaderTransferClient {
    fn new(reader: TrackingReader) -> Self {
        Self {
            reader: Arc::new(Mutex::new(Some(reader))),
        }
    }
}

impl FileTransferClient for ReaderTransferClient {
    fn put_reader<R: Read + Send + 'static>(
        &self,
        _url: &str,
        _reader: R,
        _size_bytes: u64,
    ) -> Result<(), TransferError> {
        Err(TransferError::Transport(
            "model sources do not support uploads".to_string(),
        ))
    }

    fn get_reader(&self, _url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
        self.reader
            .lock()
            .map_err(|_| TransferError::Transport("model reader state failed".to_string()))?
            .take()
            .map(|reader| Box::new(reader) as Box<dyn Read + Send>)
            .ok_or_else(|| TransferError::Transport("model reader was already opened".to_string()))
    }
}

#[derive(Default)]
struct BufferedObserver {
    started: Option<(String, Option<u64>)>,
    progress: Option<(String, u64)>,
    completed: Option<(String, u64)>,
}

impl BufferedObserver {
    fn replay<O: DownloadObserver>(self, observer: &mut O) {
        if let Some((path, expected_bytes)) = self.started {
            observer.file_started(&path, expected_bytes);
        }
        if let Some((path, downloaded_bytes)) = self.progress {
            observer.file_progress(&path, downloaded_bytes);
        }
        if let Some((path, downloaded_bytes)) = self.completed {
            observer.file_completed(&path, downloaded_bytes);
        }
    }
}

impl DownloadObserver for BufferedObserver {
    fn file_started(&mut self, rel_path: &str, expected_bytes: Option<u64>) {
        self.started = Some((rel_path.to_string(), expected_bytes));
    }

    fn file_progress(&mut self, rel_path: &str, downloaded_bytes: u64) {
        self.progress = Some((rel_path.to_string(), downloaded_bytes));
    }

    fn file_completed(&mut self, rel_path: &str, downloaded_bytes: u64) {
        self.completed = Some((rel_path.to_string(), downloaded_bytes));
    }
}
