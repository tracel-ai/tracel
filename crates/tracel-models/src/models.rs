use std::collections::HashSet;
use std::fmt;
use std::io::Read;
use std::sync::Arc;

use tracel_artifact::TransferObserver;
use tracel_artifact::bundle::{BundleDecode, BundleSink, BundleSource, FsBundle};
use tracel_artifact::download::{
    ArtifactFile, DownloadError, transfer_reader_to_sink_with_observer,
};
use tracel_artifact::normalize_checksum;

use sha2::{Digest, Sha256};
use tracel_artifact::upload::MultipartUploadSource;

use crate::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
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
    pub fn list(&self) -> Result<Vec<Model>, ModelsError> {
        self.ops.list_models()
    }

    /// Fetches one model by name.
    pub fn get(&self, name: &str) -> Result<Model, ModelsError> {
        self.ops.get_model(name)
    }

    /// Lists published versions of a model.
    pub fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        self.ops.list_versions(model)
    }

    /// Fetches one version using its opaque identity.
    pub fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        self.ops.get_version(model, id)
    }

    /// Downloads and verifies a version before copying it into a caller-owned bundle sink.
    ///
    /// Progress callbacks run synchronously while backend bytes are staged and may cancel the
    /// active transfer through [`TransferObserver::is_cancelled`]. The destination is untouched
    /// unless every file passes path, size, and checksum verification.
    pub fn download<S, O>(
        &self,
        model: &str,
        id: &VersionId,
        destination: &mut S,
        observer: &mut O,
    ) -> Result<(), ModelsError>
    where
        S: BundleSink,
        O: TransferObserver,
    {
        let staged = self.stage(model, id, observer)?;
        staged.copy_to(destination, observer)
    }

    /// Downloads, verifies, and decodes a model version using `settings`.
    ///
    /// The decoder sees the complete staged bundle only after every backend file has passed path,
    /// size, and checksum verification.
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

    /// Creates a model that versions can be published under.
    pub fn create(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        self.ops.create_model(name, description)
    }

    /// Publishes every file in `source` as a new version of `model`.
    ///
    /// Each file is measured and checksummed here, so what the backend records is what was
    /// actually read, and the same path rules that guard a download apply before anything is
    /// written. The version only becomes visible once every file has been uploaded.
    pub fn publish<S, O>(
        &self,
        model: &str,
        source: &S,
        metadata: Option<serde_json::Value>,
        observer: &mut O,
    ) -> Result<ModelVersion, ModelsError>
    where
        S: BundleSource + MultipartUploadSource,
        O: TransferObserver,
    {
        let files = measured_files(source, observer)?;
        if observer.is_cancelled() {
            return Err(ModelsError::Cancelled);
        }
        self.ops
            .publish_version(model, &files, source, metadata.as_ref(), observer)
    }

    fn stage<O: TransferObserver>(
        &self,
        model: &str,
        id: &VersionId,
        observer: &mut O,
    ) -> Result<StagedVersion, ModelsError> {
        if observer.is_cancelled() {
            return Err(ModelsError::Cancelled);
        }

        let sources = self.ops.fetch_version_files(model, id)?;
        let paths = validated_source_paths(&sources)?;
        let mut bundle = FsBundle::temp().map_err(ModelsError::other)?;

        for (source, path) in sources.iter().zip(paths) {
            stage_source(source.as_ref(), path, &mut bundle, observer)?;
        }

        Ok(StagedVersion { bundle })
    }
}

impl fmt::Debug for Models {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Models").finish_non_exhaustive()
    }
}

struct StagedVersion {
    bundle: FsBundle,
}

impl StagedVersion {
    fn copy_to<S: BundleSink, O: TransferObserver>(
        &self,
        destination: &mut S,
        observer: &mut O,
    ) -> Result<(), ModelsError> {
        for path in self.bundle.file_paths() {
            if observer.is_cancelled() {
                return Err(ModelsError::Cancelled);
            }
            let mut reader = self.bundle.open(&path).map_err(ModelsError::Output)?;
            destination
                .put_file(&path, &mut reader)
                .map_err(ModelsError::Output)?;
        }
        Ok(())
    }
}

impl BundleSource for StagedVersion {
    fn open(&self, path: &str) -> Result<Box<dyn Read + Send>, String> {
        let canonical = canonical_version_path(path).map_err(|error| error.to_string())?;
        self.bundle.open(&canonical)
    }

    fn list(&self) -> Result<Vec<String>, String> {
        self.bundle.list()
    }
}

fn stage_source<O: TransferObserver>(
    source: &dyn VersionFileSource,
    path: String,
    bundle: &mut FsBundle,
    observer: &mut O,
) -> Result<(), ModelsError> {
    let reader = source.open(&path)?;
    let file = ArtifactFile {
        rel_path: path,
        size_bytes: Some(source.file().size_bytes),
        checksum: Some(source.file().checksum.clone()),
    };

    transfer_reader_to_sink_with_observer(reader, bundle, &file, observer)
        .map_err(map_download_error)
}

/// Reads a transfer failure as the model problem it stands for.
fn map_download_error(error: DownloadError) -> ModelsError {
    match error {
        DownloadError::Cancelled { .. } => ModelsError::Cancelled,
        DownloadError::Transfer { source, .. } => ModelsError::other(source),
        DownloadError::TargetError(message) => ModelsError::Output(message),
        DownloadError::SizeMismatch {
            path,
            expected,
            actual,
        } => ModelsError::Verification {
            rel_path: path,
            problem: format!("expected {expected} bytes, got {actual}"),
        },
        DownloadError::ChecksumMismatch {
            path,
            expected,
            actual,
        } => ModelsError::Verification {
            rel_path: path,
            problem: format!("expected checksum {expected}, got {actual}"),
        },
        DownloadError::InvalidChecksum(message) => ModelsError::InvalidChecksum(message),
        DownloadError::InvalidPath(message) => ModelsError::InvalidPath(message),
    }
}

/// Measures every file a caller wants published, rejecting a listing the capability would
/// refuse to download.
fn measured_files<S: BundleSource, O: TransferObserver>(
    source: &S,
    observer: &mut O,
) -> Result<Vec<VersionFile>, ModelsError> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for path in source.list().map_err(ModelsError::Output)? {
        let canonical = canonical_version_path(&path)?;
        if !seen.insert(canonical.to_lowercase()) {
            return Err(invalid_path(format!(
                "duplicate relative model path: {canonical}"
            )));
        }
        if files
            .iter()
            .any(|file: &VersionFile| paths_conflict(&file.rel_path, &canonical))
        {
            return Err(invalid_path(format!(
                "model path conflicts with another file or directory: {canonical}"
            )));
        }

        let mut reader = source.open(&path).map_err(ModelsError::Output)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut size_bytes = 0u64;
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| ModelsError::Output(error.to_string()))?;
            if read == 0 {
                break;
            }
            if observer.is_cancelled() {
                return Err(ModelsError::Cancelled);
            }
            hasher.update(&buffer[..read]);
            size_bytes += read as u64;
        }

        files.push(VersionFile {
            rel_path: canonical,
            size_bytes,
            checksum: format!("{:x}", hasher.finalize()),
        });
    }

    Ok(files)
}

fn validated_source_paths(
    sources: &[Box<dyn VersionFileSource>],
) -> Result<Vec<String>, ModelsError> {
    let mut seen = HashSet::with_capacity(sources.len());
    let mut paths: Vec<String> = Vec::with_capacity(sources.len());

    for source in sources {
        let checksum =
            normalize_checksum(&source.file().checksum).map_err(ModelsError::InvalidChecksum)?;
        if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ModelsError::InvalidChecksum(format!(
                "expected a 64-digit SHA-256 checksum, got '{}'",
                source.file().checksum
            )));
        }
        let path = canonical_version_path(&source.file().rel_path)?;
        // Case-insensitive filesystems would let the second file overwrite the first, after
        // both passed verification.
        if !seen.insert(path.to_lowercase()) {
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
    ModelsError::InvalidPath(message)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use tracel_artifact::bundle::InMemoryBundleSources;

    use super::*;
    use crate::test_support::{FakeOps, SourceSpec, checksum, models_with_sources};

    #[derive(Default)]
    struct RecordingObserver {
        started: Vec<String>,
        progress: Vec<u64>,
        completed: Vec<u64>,
        completed_paths: Vec<String>,
    }

    impl TransferObserver for RecordingObserver {
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
    fn publish_measures_the_bytes_it_sends() {
        let ops = FakeOps::new(Vec::new());
        let record = ops.publish_record();
        let models = Models::new(Arc::new(ops));
        let mut bundle = FsBundle::temp().unwrap();
        bundle
            .put_file("weights.bin", &mut &b"payload"[..])
            .unwrap();

        models
            .publish(
                "alpha",
                &bundle,
                Some(serde_json::json!({"format": "burnpack"})),
                &mut (),
            )
            .unwrap();

        let record = record.lock().unwrap();
        assert_eq!(record.files.len(), 1);
        assert_eq!(record.files[0].rel_path, "weights.bin");
        assert_eq!(record.files[0].size_bytes, b"payload".len() as u64);
        assert_eq!(record.files[0].checksum, checksum(b"payload"));
        assert_eq!(record.uploaded, vec!["weights.bin".to_string()]);
    }

    #[test]
    fn a_cancelled_publish_sends_nothing() {
        let ops = FakeOps::new(Vec::new());
        let record = ops.publish_record();
        let models = Models::new(Arc::new(ops));
        let mut bundle = FsBundle::temp().unwrap();
        bundle
            .put_file("weights.bin", &mut &b"payload"[..])
            .unwrap();

        struct Cancelled;
        impl TransferObserver for Cancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let error = models
            .publish("alpha", &bundle, None, &mut Cancelled)
            .unwrap_err();

        assert!(error.is_cancelled());
        assert!(record.lock().unwrap().files.is_empty());
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

    #[test]
    fn midstream_source_errors_remain_transport_errors_and_leave_destination_untouched() {
        let mut source = SourceSpec::new("weights.bin", b"complete payload");
        source.failure_at = Some(4);
        source.chunk_size = 4;
        let models = models_with_sources(vec![
            SourceSpec::new("metadata.json", b"already staged"),
            source,
        ]);
        let mut destination = InMemoryBundleSources::new();

        let error = models
            .download(
                "alpha",
                &VersionId::new("version-id"),
                &mut destination,
                &mut (),
            )
            .unwrap_err();

        assert!(
            matches!(&error, ModelsError::Other(source) if source.to_string().contains("mid-stream"))
        );
        assert!(destination.is_empty());
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
    fn paths_differing_only_in_case_are_rejected_before_any_source_is_opened() {
        let sources = vec![
            SourceSpec::new("Weights.bin", b"upper"),
            SourceSpec::new("weights.bin", b"lower"),
        ];
        let opens = Arc::clone(&sources[0].opens);
        let models = models_with_sources(sources);
        let mut destination = RecordingSink::default();

        let error = models
            .download(
                "alpha",
                &VersionId::new("version-id"),
                &mut destination,
                &mut (),
            )
            .unwrap_err();

        assert!(matches!(error, ModelsError::InvalidPath(_)));
        assert!(destination.files.is_empty());
        assert_eq!(opens.load(Ordering::SeqCst), 0);
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

        assert!(matches!(error, ModelsError::InvalidChecksum(_)));
        assert_eq!(opens.load(Ordering::SeqCst), 0);
    }

    #[derive(Default)]
    struct CancellingObserver {
        cancelled: bool,
        completed: bool,
    }

    impl TransferObserver for CancellingObserver {
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
}
