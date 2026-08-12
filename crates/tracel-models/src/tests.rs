use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::Digest;
use tracel_artifact::bundle::{BundleDecode, BundleSource, InMemoryBundleSources};

use crate::{
    DownloadObserver, Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile,
    VersionFileReader, VersionFileSource, VersionId,
};

#[derive(Clone)]
struct SourceSpec {
    file: VersionFile,
    bytes: Vec<u8>,
    chunk_size: usize,
    failure_at: Option<usize>,
    open_error: Option<String>,
    opens: Arc<AtomicUsize>,
    consumed: Arc<AtomicUsize>,
    opened_paths: Arc<Mutex<Vec<String>>>,
}

impl SourceSpec {
    fn new(path: &str, bytes: &[u8]) -> Self {
        Self {
            file: VersionFile {
                rel_path: path.to_string(),
                size_bytes: bytes.len() as u64,
                checksum: checksum(bytes),
            },
            bytes: bytes.to_vec(),
            chunk_size: usize::MAX,
            failure_at: None,
            open_error: None,
            opens: Arc::new(AtomicUsize::new(0)),
            consumed: Arc::new(AtomicUsize::new(0)),
            opened_paths: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn source(&self) -> Box<dyn VersionFileSource> {
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
        if let Some(message) = &self.0.open_error {
            return Err(ModelsError::Transport(message.clone()));
        }

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

#[derive(Clone)]
struct FakeOps {
    models: Page<Model>,
    sources: Vec<SourceSpec>,
}

impl FakeOps {
    fn new(sources: Vec<SourceSpec>) -> Self {
        Self {
            models: Page {
                items: vec![model("alpha"), model("beta")],
                total: 7,
            },
            sources,
        }
    }
}

impl ModelOps for FakeOps {
    fn list_models(&self) -> Result<Page<Model>, ModelsError> {
        Ok(self.models.clone())
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.models
            .items
            .iter()
            .find(|model| model.name == name)
            .cloned()
            .ok_or_else(|| ModelsError::ModelNotFound {
                name: name.to_string(),
            })
    }

    fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        self.get_model(model)?;
        Ok(Page {
            items: Vec::new(),
            total: 0,
        })
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

fn model(name: &str) -> Model {
    Model {
        id: format!("{name}-id"),
        name: name.to_string(),
        description: None,
        published_by: Some("publisher".to_string()),
        created_at: "2026-08-12T00:00:00Z".to_string(),
        version_count: 0,
        latest_version: None,
    }
}

fn models_with_sources(sources: Vec<SourceSpec>) -> Models {
    Models::new(Arc::new(FakeOps::new(sources)))
}

fn checksum(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[derive(Default)]
struct RecordingObserver {
    started: Vec<String>,
    progress: Vec<u64>,
    completed: Vec<u64>,
    completed_paths: Vec<String>,
}

impl DownloadObserver for RecordingObserver {
    fn file_started(&mut self, rel_path: &str, _expected_bytes: Option<u64>) {
        self.started.push(rel_path.to_string());
    }

    fn file_progress(&mut self, _rel_path: &str, downloaded_bytes: u64) {
        self.progress.push(downloaded_bytes);
    }

    fn file_completed(&mut self, rel_path: &str, downloaded_bytes: u64) {
        self.completed.push(downloaded_bytes);
        self.completed_paths.push(rel_path.to_string());
    }
}

#[derive(Default)]
struct RecordingSink {
    files: Vec<(String, Vec<u8>)>,
}

impl crate::BundleSink for RecordingSink {
    fn put_file<R: Read>(&mut self, path: &str, reader: &mut R) -> Result<(), String> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        self.files.push((path.to_string(), bytes));
        Ok(())
    }
}

#[test]
fn list_preserves_backend_pagination() {
    let models = models_with_sources(vec![SourceSpec::new("weights.bin", b"payload")]);

    let page = models.list().unwrap();

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 7);
}

#[test]
fn not_found_meanings_survive_the_capability() {
    let models = models_with_sources(vec![SourceSpec::new("weights.bin", b"payload")]);

    let missing_model = models.get("missing").unwrap_err();
    let missing_version = models
        .download(
            "alpha",
            &VersionId::new("missing"),
            &mut InMemoryBundleSources::new(),
            &mut (),
        )
        .unwrap_err();

    assert!(matches!(missing_model, ModelsError::ModelNotFound { name } if name == "missing"));
    assert!(matches!(
        missing_version,
        ModelsError::VersionNotFound { model, id }
            if model == "alpha" && id == VersionId::new("missing")
    ));
    assert!(ModelsError::ScopeNotFound.is_not_found());
}

#[test]
fn session_expiry_has_a_dedicated_outcome() {
    assert!(ModelsError::SessionExpired.is_session_expired());
    assert!(!ModelsError::ScopeNotFound.is_session_expired());
}

#[test]
fn download_reports_verified_progress() {
    let bytes = b"verified payload";
    let models = models_with_sources(vec![SourceSpec::new("weights.bin", bytes)]);
    let mut destination = InMemoryBundleSources::new();
    let mut observer = RecordingObserver::default();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut observer,
        )
        .unwrap();

    assert_eq!(observer.progress.last(), Some(&(bytes.len() as u64)));
    assert_eq!(observer.completed, vec![bytes.len() as u64]);
    assert_eq!(destination.files()[0].source(), bytes);
}

#[test]
fn failed_file_set_verification_never_reaches_the_destination() {
    let valid = SourceSpec::new("first.bin", b"trusted");
    let mut invalid = SourceSpec::new("second.bin", b"untrusted");
    invalid.file.size_bytes += 1;
    let models = models_with_sources(vec![valid, invalid]);
    let mut destination = InMemoryBundleSources::new();
    let mut observer = RecordingObserver::default();

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut observer,
        )
        .unwrap_err();

    assert!(error.is_verification());
    assert!(destination.is_empty());
    assert_eq!(observer.completed_paths, vec!["first.bin"]);
}

#[derive(Debug, PartialEq)]
struct Decoded(String);

impl BundleDecode for Decoded {
    type Settings = ();
    type Error = String;

    fn decode<I: BundleSource>(
        source: &I,
        _settings: &Self::Settings,
    ) -> Result<Self, Self::Error> {
        let mut reader = source.open("weights.bin")?;
        let mut value = String::new();
        reader
            .read_to_string(&mut value)
            .map_err(|error| error.to_string())?;
        Ok(Self(value))
    }
}

#[test]
fn load_decodes_only_after_complete_verification() {
    let models = models_with_sources(vec![SourceSpec::new("weights.bin", b"decoded")]);

    let decoded = models
        .load::<Decoded>("alpha", &VersionId::new("version-id"), &())
        .unwrap();

    assert_eq!(decoded, Decoded("decoded".to_string()));
}

#[derive(Debug)]
struct FailingDecode;

impl BundleDecode for FailingDecode {
    type Settings = ();
    type Error = String;

    fn decode<I: BundleSource>(
        _source: &I,
        _settings: &Self::Settings,
    ) -> Result<Self, Self::Error> {
        Err("unsupported model format".to_string())
    }
}

#[test]
fn load_reports_decode_failures_after_verification() {
    let models = models_with_sources(vec![SourceSpec::new("weights.bin", b"verified")]);

    let error = models
        .load::<FailingDecode>("alpha", &VersionId::new("version-id"), &())
        .unwrap_err();

    assert!(matches!(error, ModelsError::Decode(message) if message == "unsupported model format"));
}

#[test]
fn source_open_errors_preserve_transport_meaning() {
    let mut source = SourceSpec::new("weights.bin", b"payload");
    source.open_error = Some("source is offline".to_string());
    let models = models_with_sources(vec![source]);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut InMemoryBundleSources::new(),
            &mut (),
        )
        .unwrap_err();

    assert!(matches!(error, ModelsError::Transport(message) if message == "source is offline"));
}

#[test]
fn midstream_source_errors_remain_transport_errors() {
    let mut source = SourceSpec::new("weights.bin", b"complete payload");
    source.failure_at = Some(4);
    source.chunk_size = 4;
    let models = models_with_sources(vec![source]);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut InMemoryBundleSources::new(),
            &mut (),
        )
        .unwrap_err();

    assert!(matches!(error, ModelsError::Transport(message) if message.contains("mid-stream")));
}

#[test]
fn verified_canonical_paths_are_delivered_to_the_destination() {
    let source = SourceSpec::new("weights\\./model.bin", b"canonical");
    let opened_paths = Arc::clone(&source.opened_paths);
    let models = models_with_sources(vec![source]);
    let mut destination = RecordingSink::default();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut (),
        )
        .unwrap();

    assert_eq!(
        destination.files,
        vec![("weights/model.bin".to_string(), b"canonical".to_vec())]
    );
    assert_eq!(
        opened_paths.lock().unwrap().as_slice(),
        &["weights/model.bin"]
    );
}

#[test]
fn case_distinct_paths_retain_their_own_verified_bytes() {
    let sources = vec![
        SourceSpec::new("Weights.bin", b"upper"),
        SourceSpec::new("weights.bin", b"lower"),
    ];
    let models = models_with_sources(sources);
    let mut destination = RecordingSink::default();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut (),
        )
        .unwrap();

    assert_eq!(
        destination.files,
        vec![
            ("Weights.bin".to_string(), b"upper".to_vec()),
            ("weights.bin".to_string(), b"lower".to_vec()),
        ]
    );
}

#[test]
fn aliases_are_rejected_before_any_source_is_opened() {
    let first = SourceSpec::new("weights/model.bin", b"first");
    let second = SourceSpec::new("weights//model.bin", b"second");
    let opens = [Arc::clone(&first.opens), Arc::clone(&second.opens)];
    let models = models_with_sources(vec![first, second]);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut RecordingSink::default(),
            &mut (),
        )
        .unwrap_err();

    assert!(error.is_verification());
    assert!(opens.iter().all(|opens| opens.load(Ordering::SeqCst) == 0));
}

#[test]
fn file_and_directory_conflicts_are_rejected_before_opening_sources() {
    for paths in [
        ["weights", "weights/model.bin"],
        ["weights/model.bin", "weights"],
    ] {
        let first = SourceSpec::new(paths[0], b"first");
        let second = SourceSpec::new(paths[1], b"second");
        let opens = [Arc::clone(&first.opens), Arc::clone(&second.opens)];
        let models = models_with_sources(vec![first, second]);

        let error = models
            .download(
                "alpha",
                &VersionId::new("version-id"),
                &mut RecordingSink::default(),
                &mut (),
            )
            .unwrap_err();

        assert!(error.is_verification());
        assert!(opens.iter().all(|opens| opens.load(Ordering::SeqCst) == 0));
    }
}

#[test]
fn unsafe_and_nonportable_paths_are_verification_errors() {
    for path in [
        "/absolute.bin",
        "../escape.bin",
        "C:weights.bin",
        "NUL",
        "aux.txt",
        "weights.bin.",
        "weights.bin ",
        "bad\0name.bin",
    ] {
        let models = models_with_sources(vec![SourceSpec::new(path, b"payload")]);

        let error = models
            .download(
                "alpha",
                &VersionId::new("version-id"),
                &mut RecordingSink::default(),
                &mut (),
            )
            .unwrap_err();

        assert!(
            error.is_verification(),
            "unexpected error for {path:?}: {error}"
        );
    }
}

#[test]
fn malformed_checksum_is_rejected_before_the_source_opens() {
    let mut source = SourceSpec::new("weights.bin", b"payload");
    source.file.checksum = "not-a-sha256".to_string();
    let opens = Arc::clone(&source.opens);
    let models = models_with_sources(vec![source]);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut RecordingSink::default(),
            &mut (),
        )
        .unwrap_err();

    assert!(matches!(
        error,
        ModelsError::Verification(tracel_artifact::download::DownloadError::InvalidChecksum(_))
    ));
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

#[derive(Default)]
struct CancellingObserver {
    cancelled: bool,
    completed: bool,
}

impl DownloadObserver for CancellingObserver {
    fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    fn file_progress(&mut self, _rel_path: &str, _downloaded_bytes: u64) {
        self.cancelled = true;
    }

    fn file_completed(&mut self, _rel_path: &str, _downloaded_bytes: u64) {
        self.completed = true;
    }
}

#[test]
fn cancellation_reaches_the_active_source_transfer() {
    let mut source = SourceSpec::new("weights.bin", b"a payload spanning several reads");
    source.chunk_size = 4;
    let consumed = Arc::clone(&source.consumed);
    let total = source.bytes.len();
    let models = models_with_sources(vec![source]);
    let mut destination = InMemoryBundleSources::new();
    let mut observer = CancellingObserver::default();

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut observer,
        )
        .unwrap_err();

    assert!(error.is_cancelled());
    assert_eq!(consumed.load(Ordering::SeqCst), 4);
    assert!(consumed.load(Ordering::SeqCst) < total);
    assert!(!observer.completed);
    assert!(destination.is_empty());
}

#[test]
fn model_ops_and_file_sources_are_object_safe() {
    let source = SourceSpec::new("weights.bin", b"payload").source();
    let _: &dyn VersionFileSource = source.as_ref();

    let ops: Arc<dyn ModelOps> = Arc::new(FakeOps::new(vec![SourceSpec::new(
        "weights.bin",
        b"payload",
    )]));
    let _models = Models::new(ops);
}
