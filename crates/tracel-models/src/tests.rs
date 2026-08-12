use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::Digest;
use tracel_artifact::bundle::{BundleDecode, BundleSource, InMemoryBundleSources};

use crate::{
    DownloadObserver, Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile,
    VersionFileSource, VersionId,
};

#[derive(Clone)]
struct FakeOps {
    models: Page<Model>,
    files: Arc<HashMap<String, Vec<u8>>>,
    invalid_size: bool,
}

impl FakeOps {
    fn new(bytes: &[u8]) -> Self {
        Self {
            models: Page {
                items: vec![model("alpha"), model("beta")],
                total: 7,
            },
            files: Arc::new(HashMap::from([("weights.bin".to_string(), bytes.to_vec())])),
            invalid_size: false,
        }
    }

    fn with_invalid_size(mut self) -> Self {
        self.invalid_size = true;
        self
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
    ) -> Result<Vec<VersionFileSource>, ModelsError> {
        self.get_model(model)?;
        if id.as_str() != "version-id" {
            return Err(ModelsError::VersionNotFound {
                model: model.to_string(),
                id: id.clone(),
            });
        }

        let bytes = self.files["weights.bin"].clone();
        let expected_size = bytes.len() as u64 + u64::from(self.invalid_size);
        let file = VersionFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: expected_size,
            checksum: checksum(&bytes),
        };
        Ok(vec![VersionFileSource::new(file, move || {
            Ok(Box::new(Cursor::new(bytes.clone())))
        })])
    }
}

fn model(name: &str) -> Model {
    Model {
        id: format!("{name}-id"),
        name: name.to_string(),
        description: None,
        created_at: "2026-08-12T00:00:00Z".to_string(),
        version_count: 0,
        latest_version: None,
    }
}

#[derive(Clone)]
struct SourceOps {
    sources: Vec<VersionFileSource>,
}

impl ModelOps for SourceOps {
    fn list_models(&self) -> Result<Page<Model>, ModelsError> {
        Ok(Page {
            items: vec![model("alpha")],
            total: 1,
        })
    }

    fn get_model(&self, _name: &str) -> Result<Model, ModelsError> {
        Ok(model("alpha"))
    }

    fn list_versions(&self, _model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        Ok(Page {
            items: Vec::new(),
            total: 0,
        })
    }

    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        Err(ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        })
    }

    fn fetch_version_files(
        &self,
        _model: &str,
        _id: &VersionId,
    ) -> Result<Vec<VersionFileSource>, ModelsError> {
        Ok(self.sources.clone())
    }
}

fn models_with_source(source: VersionFileSource) -> Models {
    models_with_sources(vec![source])
}

fn models_with_sources(sources: Vec<VersionFileSource>) -> Models {
    Models::new(Arc::new(SourceOps { sources }))
}

fn described_file(bytes: &[u8]) -> VersionFile {
    described_file_at("weights.bin", bytes)
}

fn described_file_at(path: &str, bytes: &[u8]) -> VersionFile {
    VersionFile {
        rel_path: path.to_string(),
        size_bytes: bytes.len() as u64,
        checksum: checksum(bytes),
    }
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

#[test]
fn list_preserves_backend_pagination() {
    let models = Models::new(Arc::new(FakeOps::new(b"payload")));

    let page = models.list().unwrap();

    assert_eq!(page.items.len(), 2);
    assert_eq!(page.total, 7);
}

#[test]
fn not_found_meanings_survive_the_capability() {
    let models = Models::new(Arc::new(FakeOps::new(b"payload")));

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
fn download_reports_verified_progress() {
    let bytes = b"verified payload";
    let models = Models::new(Arc::new(FakeOps::new(bytes)));
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
fn failed_verification_never_reaches_the_destination() {
    let models = Models::new(Arc::new(
        FakeOps::new(b"untrusted payload").with_invalid_size(),
    ));
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
    assert!(observer.completed.is_empty());
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
fn load_decodes_only_after_verification() {
    let models = Models::new(Arc::new(FakeOps::new(b"decoded")));

    let decoded = models
        .load::<Decoded>("alpha", &VersionId::new("version-id"), &())
        .unwrap();

    assert_eq!(decoded, Decoded("decoded".to_string()));
}

#[test]
fn source_transport_errors_preserve_their_meaning() {
    let source = VersionFileSource::new(described_file(b"payload"), || {
        Err(ModelsError::Transport("source is offline".to_string()))
    });
    let models = models_with_source(source);
    let mut destination = InMemoryBundleSources::new();

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut (),
        )
        .unwrap_err();

    assert!(matches!(error, ModelsError::Transport(message) if message == "source is offline"));
    assert!(destination.is_empty());
}

struct FailingReader {
    bytes: Option<Vec<u8>>,
}

impl FailingReader {
    fn after(bytes: &[u8]) -> Self {
        Self {
            bytes: Some(bytes.to_vec()),
        }
    }
}

impl Read for FailingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if let Some(bytes) = self.bytes.take() {
            let len = bytes.len().min(buffer.len());
            buffer[..len].copy_from_slice(&bytes[..len]);
            return Ok(len);
        }

        Err(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "source failed mid-stream",
        ))
    }
}

#[test]
fn midstream_source_errors_remain_transport_errors() {
    let bytes = b"partial payload";
    let source = VersionFileSource::new(described_file(bytes), move || {
        Ok(Box::new(FailingReader::after(bytes)))
    });
    let models = models_with_source(source);

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
fn verified_canonical_paths_are_delivered_to_the_destination() {
    let bytes = b"canonical".to_vec();
    let source = VersionFileSource::new(described_file_at("weights\\./model.bin", &bytes), {
        let bytes = bytes.clone();
        move || Ok(Box::new(Cursor::new(bytes.clone())))
    });
    let models = models_with_source(source);
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
        vec![("weights/model.bin".to_string(), bytes)]
    );
}

#[test]
fn case_distinct_paths_retain_their_own_verified_bytes() {
    let sources = [
        ("Weights.bin", b"upper".as_slice()),
        ("weights.bin", b"lower".as_slice()),
    ]
    .into_iter()
    .map(|(path, bytes)| {
        let bytes = bytes.to_vec();
        VersionFileSource::new(described_file_at(path, &bytes), {
            let bytes = bytes.clone();
            move || Ok(Box::new(Cursor::new(bytes.clone())))
        })
    })
    .collect();
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
fn backend_caches_receive_only_the_verified_canonical_path() {
    let bytes = b"canonical".to_vec();
    let stored_path = Arc::new(Mutex::new(None::<String>));
    let source = VersionFileSource::new(described_file_at("weights\\./model.bin", &bytes), {
        let bytes = bytes.clone();
        move || Ok(Box::new(Cursor::new(bytes.clone())))
    })
    .with_cache(
        |_| Ok(None),
        {
            let stored_path = Arc::clone(&stored_path);
            move |path, _| {
                *stored_path.lock().unwrap() = Some(path.to_string());
                Ok(())
            }
        },
        |_| Ok(()),
    );
    let models = models_with_source(source);

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut RecordingSink::default(),
            &mut (),
        )
        .unwrap();

    assert_eq!(
        stored_path.lock().unwrap().as_deref(),
        Some("weights/model.bin")
    );
}

#[test]
fn path_aliases_are_rejected_before_any_source_is_opened() {
    let bytes = b"payload".to_vec();
    let opens = Arc::new(AtomicUsize::new(0));
    let sources = ["weights/model.bin", "weights//model.bin"]
        .into_iter()
        .map(|path| {
            let bytes = bytes.clone();
            let opens = Arc::clone(&opens);
            VersionFileSource::new(described_file_at(path, &bytes), move || {
                opens.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Cursor::new(bytes.clone())))
            })
        })
        .collect();
    let models = models_with_sources(sources);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut RecordingSink::default(),
            &mut (),
        )
        .unwrap_err();

    assert!(error.is_verification());
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

#[test]
fn file_and_directory_path_conflicts_are_rejected_before_opening_sources() {
    for paths in [
        ["weights", "weights/model.bin"],
        ["weights/model.bin", "weights"],
    ] {
        let opens = Arc::new(AtomicUsize::new(0));
        let sources = paths
            .into_iter()
            .map(|path| {
                let opens = Arc::clone(&opens);
                VersionFileSource::new(described_file_at(path, b"payload"), move || {
                    opens.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(Cursor::new(b"payload".to_vec())))
                })
            })
            .collect();
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

        assert!(error.is_verification());
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        assert!(destination.files.is_empty());
    }
}

#[test]
fn absolute_and_parent_paths_are_verification_errors() {
    for path in ["/absolute.bin", "../escape.bin"] {
        let source = VersionFileSource::new(described_file_at(path, b"payload"), || {
            Ok(Box::new(Cursor::new(b"payload".to_vec())))
        });
        let models = models_with_source(source);

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
            "unexpected error for {path}: {error}"
        );
    }
}

#[test]
fn nonportable_paths_are_verification_errors() {
    for path in [
        "C:weights.bin",
        "NUL",
        "aux.txt",
        "weights.bin.",
        "weights.bin ",
        "bad\0name.bin",
    ] {
        let source = VersionFileSource::new(described_file_at(path, b"payload"), || {
            Ok(Box::new(Cursor::new(b"payload".to_vec())))
        });
        let models = models_with_source(source);

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
    let models = Models::new(Arc::new(FakeOps::new(b"verified")));

    let error = models
        .load::<FailingDecode>("alpha", &VersionId::new("version-id"), &())
        .unwrap_err();

    assert!(matches!(error, ModelsError::Decode(message) if message == "unsupported model format"));
}

#[test]
fn fetched_bytes_enter_the_backend_cache_only_after_verification() {
    let bytes = b"verified cache value".to_vec();
    let cache = Arc::new(Mutex::new(None::<Vec<u8>>));
    let stores = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(described_file(&bytes), {
        let bytes = bytes.clone();
        move || Ok(Box::new(Cursor::new(bytes.clone())))
    })
    .with_cache(
        {
            let cache = Arc::clone(&cache);
            move |_| {
                Ok(cache
                    .lock()
                    .unwrap()
                    .clone()
                    .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read + Send>))
            }
        },
        {
            let cache = Arc::clone(&cache);
            let stores = Arc::clone(&stores);
            move |_, reader| {
                let mut bytes = Vec::new();
                reader
                    .read_to_end(&mut bytes)
                    .map_err(|error| ModelsError::Cache(error.to_string()))?;
                *cache.lock().unwrap() = Some(bytes);
                stores.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
        |_| Ok(()),
    );
    let models = models_with_source(source);

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut InMemoryBundleSources::new(),
            &mut (),
        )
        .unwrap();

    assert_eq!(*cache.lock().unwrap(), Some(bytes));
    assert_eq!(stores.load(Ordering::SeqCst), 1);
}

#[test]
fn a_verified_cache_hit_avoids_the_authoritative_source() {
    let bytes = b"cached value".to_vec();
    let fetches = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(described_file(&bytes), {
        let fetches = Arc::clone(&fetches);
        move || {
            fetches.fetch_add(1, Ordering::SeqCst);
            Err(ModelsError::Transport(
                "source should not be read".to_string(),
            ))
        }
    })
    .with_cache(
        {
            let bytes = bytes.clone();
            move |_| Ok(Some(Box::new(Cursor::new(bytes.clone()))))
        },
        |_, _| Ok(()),
        |_| Ok(()),
    );
    let models = models_with_source(source);
    let mut destination = InMemoryBundleSources::new();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut (),
        )
        .unwrap();

    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    assert_eq!(destination.files()[0].source(), bytes);
}

#[test]
fn a_corrupt_cache_entry_is_invalidated_and_refetched() {
    let verified = b"authoritative value".to_vec();
    let cache = Arc::new(Mutex::new(Some(b"corrupt".to_vec())));
    let fetches = Arc::new(AtomicUsize::new(0));
    let invalidations = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(described_file(&verified), {
        let verified = verified.clone();
        let fetches = Arc::clone(&fetches);
        move || {
            fetches.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Cursor::new(verified.clone())))
        }
    })
    .with_cache(
        {
            let cache = Arc::clone(&cache);
            move |_| {
                Ok(cache
                    .lock()
                    .unwrap()
                    .clone()
                    .map(|bytes| Box::new(Cursor::new(bytes)) as Box<dyn Read + Send>))
            }
        },
        {
            let cache = Arc::clone(&cache);
            move |_, reader| {
                let mut bytes = Vec::new();
                reader
                    .read_to_end(&mut bytes)
                    .map_err(|error| ModelsError::Cache(error.to_string()))?;
                *cache.lock().unwrap() = Some(bytes);
                Ok(())
            }
        },
        {
            let cache = Arc::clone(&cache);
            let invalidations = Arc::clone(&invalidations);
            move |_| {
                *cache.lock().unwrap() = None;
                invalidations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
    );
    let models = models_with_source(source);
    let mut destination = InMemoryBundleSources::new();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut (),
        )
        .unwrap();

    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(invalidations.load(Ordering::SeqCst), 1);
    assert_eq!(*cache.lock().unwrap(), Some(verified.clone()));
    assert_eq!(destination.files()[0].source(), verified);
}

#[test]
fn malformed_checksum_does_not_invalidate_or_refetch_cache_content() {
    let cache_opens = Arc::new(AtomicUsize::new(0));
    let fetches = Arc::new(AtomicUsize::new(0));
    let invalidations = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(
        VersionFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: 7,
            checksum: "not-a-sha256".to_string(),
        },
        {
            let fetches = Arc::clone(&fetches);
            move || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Cursor::new(b"payload".to_vec())))
            }
        },
    )
    .with_cache(
        {
            let cache_opens = Arc::clone(&cache_opens);
            move |_| {
                cache_opens.fetch_add(1, Ordering::SeqCst);
                Ok(Some(Box::new(Cursor::new(b"payload".to_vec()))))
            }
        },
        |_, _| Ok(()),
        {
            let invalidations = Arc::clone(&invalidations);
            move |_| {
                invalidations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
    );
    let models = models_with_source(source);

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
    assert_eq!(cache_opens.load(Ordering::SeqCst), 0);
    assert_eq!(fetches.load(Ordering::SeqCst), 0);
    assert_eq!(invalidations.load(Ordering::SeqCst), 0);
}

#[test]
fn a_cached_reader_that_fails_midstream_is_invalidated_and_refetched() {
    let verified = b"authoritative value".to_vec();
    let fetches = Arc::new(AtomicUsize::new(0));
    let invalidations = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(described_file(&verified), {
        let verified = verified.clone();
        let fetches = Arc::clone(&fetches);
        move || {
            fetches.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Cursor::new(verified.clone())))
        }
    })
    .with_cache(
        {
            let verified = verified.clone();
            move |_| Ok(Some(Box::new(FailingReader::after(&verified))))
        },
        |_, _| Ok(()),
        {
            let invalidations = Arc::clone(&invalidations);
            move |_| {
                invalidations.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
    );
    let models = models_with_source(source);
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

    assert_eq!(fetches.load(Ordering::SeqCst), 1);
    assert_eq!(invalidations.load(Ordering::SeqCst), 1);
    assert_eq!(observer.started, vec!["weights.bin"]);
    assert_eq!(observer.completed_paths, vec!["weights.bin"]);
    assert_eq!(destination.files()[0].source(), verified);
}

#[test]
fn multiple_corrupt_cache_entries_are_refetched_without_duplicate_progress() {
    let fetches = Arc::new(AtomicUsize::new(0));
    let invalidations = Arc::new(AtomicUsize::new(0));
    let sources = [
        ("first.bin", b"first".as_slice()),
        ("second.bin", b"second".as_slice()),
    ]
    .into_iter()
    .map(|(path, bytes)| {
        let bytes = bytes.to_vec();
        VersionFileSource::new(described_file_at(path, &bytes), {
            let bytes = bytes.clone();
            let fetches = Arc::clone(&fetches);
            move || {
                fetches.fetch_add(1, Ordering::SeqCst);
                Ok(Box::new(Cursor::new(bytes.clone())))
            }
        })
        .with_cache(
            |_| Ok(Some(Box::new(Cursor::new(b"corrupt".to_vec())))),
            |_, _| Ok(()),
            {
                let invalidations = Arc::clone(&invalidations);
                move |_| {
                    invalidations.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        )
    })
    .collect();
    let models = models_with_sources(sources);
    let mut destination = RecordingSink::default();
    let mut observer = RecordingObserver::default();

    models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut destination,
            &mut observer,
        )
        .unwrap();

    assert_eq!(fetches.load(Ordering::SeqCst), 2);
    assert_eq!(invalidations.load(Ordering::SeqCst), 2);
    assert_eq!(observer.started, vec!["first.bin", "second.bin"]);
    assert_eq!(observer.completed_paths, vec!["first.bin", "second.bin"]);
    assert_eq!(destination.files.len(), 2);
}

#[test]
fn unverified_bytes_never_enter_the_backend_cache() {
    let expected = b"expected value";
    let stores = Arc::new(AtomicUsize::new(0));
    let source = VersionFileSource::new(described_file(expected), || {
        Ok(Box::new(Cursor::new(b"different value".to_vec())))
    })
    .with_cache(
        |_| Ok(None),
        {
            let stores = Arc::clone(&stores);
            move |_, _| {
                stores.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
        |_| Ok(()),
    );
    let models = models_with_source(source);

    let error = models
        .download(
            "alpha",
            &VersionId::new("version-id"),
            &mut InMemoryBundleSources::new(),
            &mut (),
        )
        .unwrap_err();

    assert!(error.is_verification());
    assert_eq!(stores.load(Ordering::SeqCst), 0);
}

#[test]
fn model_ops_is_object_safe() {
    let ops: Arc<dyn ModelOps> = Arc::new(FakeOps::new(b"payload"));
    let _models = Models::new(ops);
}
