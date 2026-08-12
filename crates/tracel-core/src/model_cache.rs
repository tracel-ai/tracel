//! Backend-owned local cache primitives for verified model files.

use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use sha2::Digest;
use tracel_artifact::{FileTransferClient, normalize_checksum};
use tracel_models::{ModelsError, VersionFile, VersionFileReader, VersionFileSource, VersionId};

#[derive(Debug, Clone)]
pub(crate) struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn file_source<FTC: FileTransferClient>(
        &self,
        model: &str,
        id: &VersionId,
        file: VersionFile,
        url: String,
        transfer_client: FTC,
    ) -> CachedVersionFileSource<FTC> {
        CachedVersionFileSource {
            cache: self.clone(),
            model: model.to_string(),
            id: id.clone(),
            file,
            url,
            transfer_client,
        }
    }

    #[cfg(test)]
    fn open(
        &self,
        model: &str,
        id: &VersionId,
        rel_path: &str,
    ) -> Result<Option<VersionFileReader>, ModelsError> {
        let path = self.file_path(model, id, rel_path)?;
        match File::open(path) {
            Ok(file) => Ok(Some(Box::new(file))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ModelsError::Cache(error.to_string())),
        }
    }

    #[cfg(test)]
    fn store(
        &self,
        model: &str,
        id: &VersionId,
        rel_path: &str,
        reader: &mut dyn Read,
    ) -> Result<(), ModelsError> {
        let destination = self.file_path(model, id, rel_path)?;
        let parent = destination
            .parent()
            .ok_or_else(|| ModelsError::Cache("model cache path has no parent".to_string()))?;
        std::fs::create_dir_all(parent).map_err(|error| ModelsError::Cache(error.to_string()))?;

        let mut staged = tempfile::Builder::new()
            .prefix(".tracel-model-")
            .tempfile_in(parent)
            .map_err(|error| ModelsError::Cache(error.to_string()))?;
        std::io::copy(reader, staged.as_file_mut())
            .map_err(|error| ModelsError::Cache(error.to_string()))?;
        staged
            .as_file_mut()
            .flush()
            .map_err(|error| ModelsError::Cache(error.to_string()))?;
        staged
            .as_file()
            .sync_all()
            .map_err(|error| ModelsError::Cache(error.to_string()))?;
        staged
            .persist(destination)
            .map_err(|error| ModelsError::Cache(error.error.to_string()))?;
        Ok(())
    }

    #[cfg(test)]
    fn invalidate(&self, model: &str, id: &VersionId, rel_path: &str) -> Result<(), ModelsError> {
        let path = self.file_path(model, id, rel_path)?;
        remove_cache_file(&path)
    }

    fn file_path(
        &self,
        model: &str,
        id: &VersionId,
        rel_path: &str,
    ) -> Result<PathBuf, ModelsError> {
        validate_relative_path(rel_path)?;
        Ok(self.version_dir(model, id).join(opaque_cache_key(rel_path)))
    }

    fn version_dir(&self, model: &str, id: &VersionId) -> PathBuf {
        self.root
            .join(opaque_cache_key(model))
            .join(opaque_cache_key(id.as_str()))
    }

    fn open_verified(
        &self,
        model: &str,
        id: &VersionId,
        canonical_path: &str,
        descriptor: &VersionFile,
    ) -> Result<Option<CacheReader>, ModelsError> {
        let path = self.file_path(model, id, canonical_path)?;
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(ModelsError::Cache(error.to_string())),
        };

        // Cache admission is checked here so corrupt bytes can fall back before the capability
        // consumes them; Models still performs the authoritative verification before delivery.
        let valid = cache_file_matches(&mut file, descriptor)
            .map_err(|error| ModelsError::Cache(error.to_string()))?;
        if valid {
            file.rewind()
                .map_err(|error| ModelsError::Cache(error.to_string()))?;
            return Ok(Some(CacheReader(file)));
        }

        // The corrupt entry stays in place until a completely verified candidate atomically
        // replaces it. Removing it here could race another process publishing a valid repair.
        Ok(None)
    }

    fn candidate(
        &self,
        model: &str,
        id: &VersionId,
        canonical_path: &str,
        descriptor: &VersionFile,
    ) -> Result<CacheCandidate, ModelsError> {
        let destination = self.file_path(model, id, canonical_path)?;
        let parent = destination
            .parent()
            .ok_or_else(|| ModelsError::Cache("model cache path has no parent".to_string()))?;
        std::fs::create_dir_all(parent).map_err(|error| ModelsError::Cache(error.to_string()))?;
        let staged = tempfile::Builder::new()
            .prefix(".tracel-model-")
            .tempfile_in(parent)
            .map_err(|error| ModelsError::Cache(error.to_string()))?;

        Ok(CacheCandidate {
            staged: Some(staged),
            destination,
            expected_size: descriptor.size_bytes,
            expected_checksum: normalize_checksum(&descriptor.checksum)
                .map_err(|error| ModelsError::Cache(error.to_string()))?,
            hasher: sha2::Sha256::new(),
            total: 0,
        })
    }
}

#[cfg(test)]
fn remove_cache_file(path: &Path) -> Result<(), ModelsError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ModelsError::Cache(error.to_string())),
    }
}

fn cache_file_matches(file: &mut File, descriptor: &VersionFile) -> Result<bool, std::io::Error> {
    let mut hasher = sha2::Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }

    let expected_checksum = normalize_checksum(&descriptor.checksum)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(total == descriptor.size_bytes && format!("{:x}", hasher.finalize()) == expected_checksum)
}

struct CacheReader(File);

impl Read for CacheReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buffer).map_err(cache_io_error)
    }
}

pub(crate) struct CachedVersionFileSource<FTC> {
    cache: ModelCache,
    model: String,
    id: VersionId,
    file: VersionFile,
    url: String,
    transfer_client: FTC,
}

impl<FTC: FileTransferClient> VersionFileSource for CachedVersionFileSource<FTC> {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        if let Some(file) =
            self.cache
                .open_verified(&self.model, &self.id, canonical_path, &self.file)?
        {
            return Ok(Box::new(file));
        }

        let candidate = self
            .cache
            .candidate(&self.model, &self.id, canonical_path, &self.file)?;
        let reader = self
            .transfer_client
            .get_reader(&self.url)
            .map_err(|error| ModelsError::Transport(error.to_string()))?;
        Ok(Box::new(CachingReader {
            inner: reader,
            candidate: Some(candidate),
        }))
    }
}

struct CachingReader {
    inner: VersionFileReader,
    candidate: Option<CacheCandidate>,
}

impl Read for CachingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        let Some(candidate) = &mut self.candidate else {
            return Ok(read);
        };

        if read == 0 {
            let candidate = self.candidate.take().expect("cache candidate is present");
            candidate.publish_if_verified()?;
            return Ok(0);
        }

        candidate.write(&buffer[..read])?;
        Ok(read)
    }
}

struct CacheCandidate {
    staged: Option<tempfile::NamedTempFile>,
    destination: PathBuf,
    expected_size: u64,
    expected_checksum: String,
    hasher: sha2::Sha256,
    total: u64,
}

impl CacheCandidate {
    fn write(&mut self, bytes: &[u8]) -> Result<(), std::io::Error> {
        let staged = self.staged.as_mut().expect("cache candidate is active");
        staged.write_all(bytes).map_err(cache_io_error)?;
        self.hasher.update(bytes);
        self.total += bytes.len() as u64;
        Ok(())
    }

    fn publish_if_verified(mut self) -> Result<(), std::io::Error> {
        let checksum = format!("{:x}", self.hasher.finalize());
        if self.total != self.expected_size || checksum != self.expected_checksum {
            return Ok(());
        }

        let Some(mut staged) = self.staged.take() else {
            return Ok(());
        };
        staged.as_file_mut().flush().map_err(cache_io_error)?;
        staged.as_file().sync_all().map_err(cache_io_error)?;
        staged
            .persist(&self.destination)
            .map_err(|error| cache_io_error(error.error))?;
        Ok(())
    }
}

fn cache_io_error(error: std::io::Error) -> std::io::Error {
    std::io::Error::other(ModelsError::Cache(error.to_string()))
}

pub(crate) fn opaque_cache_key(value: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn validate_relative_path(rel_path: &str) -> Result<(), ModelsError> {
    let path = Path::new(rel_path);
    if !rel_path.is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(ModelsError::Cache(format!(
            "invalid model cache path: {rel_path}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use tracel_artifact::TransferError;

    #[test]
    fn verified_file_can_be_stored_opened_and_invalidated() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");

        cache
            .store(
                "mnist",
                &id,
                "weights/model.bin",
                &mut Cursor::new(b"weights"),
            )
            .unwrap();

        let mut reader = cache
            .open("mnist", &id, "weights/model.bin")
            .unwrap()
            .expect("expected cache hit");
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"weights");

        cache.invalidate("mnist", &id, "weights/model.bin").unwrap();
        assert!(
            cache
                .open("mnist", &id, "weights/model.bin")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn opaque_version_identity_scopes_cache_entries() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let first = VersionId::new("first-id");
        let second = VersionId::new("second-id");

        cache
            .store("mnist", &first, "weights.bin", &mut Cursor::new(b"first"))
            .unwrap();

        assert!(
            cache
                .open("mnist", &second, "weights.bin")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn cache_file_paths_cannot_escape_the_cache_root() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");

        assert!(cache.open("mnist", &id, "../weights.bin").is_err());
        assert!(cache.open("mnist", &id, "/tmp/weights.bin").is_err());
    }

    #[test]
    fn cache_paths_do_not_alias_under_filesystem_case_rules() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");

        cache
            .store("mnist", &id, "Weights.bin", &mut Cursor::new(b"upper"))
            .unwrap();
        cache
            .store("mnist", &id, "weights.bin", &mut Cursor::new(b"lower"))
            .unwrap();

        for (path, expected) in [("Weights.bin", b"upper"), ("weights.bin", b"lower")] {
            let mut reader = cache.open("mnist", &id, path).unwrap().unwrap();
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, expected);
        }
    }

    #[test]
    fn concurrent_stores_publish_one_complete_verified_file() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let stores = [b"first".to_vec(), b"second".to_vec()]
            .into_iter()
            .map(|bytes| {
                let cache = cache.clone();
                let id = id.clone();
                std::thread::spawn(move || {
                    cache
                        .store("mnist", &id, "weights.bin", &mut Cursor::new(bytes))
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();

        for store in stores {
            store.join().unwrap();
        }

        let mut reader = cache
            .open("mnist", &id, "weights.bin")
            .unwrap()
            .expect("expected cache hit");
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert!(bytes == b"first" || bytes == b"second");
    }

    #[derive(Clone)]
    struct FakeTransferClient {
        bytes: Arc<Vec<u8>>,
        fetches: Arc<AtomicUsize>,
    }

    impl FileTransferClient for FakeTransferClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            _reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            unreachable!("model cache tests only download")
        }

        fn get_reader(&self, _url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            self.fetches.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Cursor::new(self.bytes.as_ref().clone())))
        }
    }

    fn descriptor(bytes: &[u8]) -> VersionFile {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        VersionFile {
            rel_path: "weights.bin".to_string(),
            size_bytes: bytes.len() as u64,
            checksum: format!("{:x}", hasher.finalize()),
        }
    }

    fn fake_transfer(bytes: &[u8]) -> (FakeTransferClient, Arc<AtomicUsize>) {
        let fetches = Arc::new(AtomicUsize::new(0));
        (
            FakeTransferClient {
                bytes: Arc::new(bytes.to_vec()),
                fetches: Arc::clone(&fetches),
            },
            fetches,
        )
    }

    #[test]
    fn streamed_verified_file_is_reused_without_another_transfer() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let bytes = b"verified bytes";
        let file = descriptor(bytes);
        let (transfer, fetches) = fake_transfer(bytes);

        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer.clone(),
        );
        let mut reader = source.open("weights.bin").unwrap();
        let mut first = Vec::new();
        reader.read_to_end(&mut first).unwrap();
        drop(reader);

        let source = cache.file_source("mnist", &id, file, "mock://weights".to_string(), transfer);
        let mut reader = source.open("weights.bin").unwrap();
        let mut second = Vec::new();
        reader.read_to_end(&mut second).unwrap();

        assert_eq!(first, bytes);
        assert_eq!(second, bytes);
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn corrupt_cache_entry_is_refetched_and_atomically_replaced_inside_the_source() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let verified = b"authoritative bytes";
        let file = descriptor(verified);
        cache
            .store("mnist", &id, &file.rel_path, &mut Cursor::new(b"corrupt"))
            .unwrap();
        let (transfer, fetches) = fake_transfer(verified);
        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer.clone(),
        );

        let mut reader = source.open("weights.bin").unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();

        let source = cache.file_source("mnist", &id, file, "mock://weights".to_string(), transfer);
        let mut reader = source.open("weights.bin").unwrap();
        let mut cached = Vec::new();
        reader.read_to_end(&mut cached).unwrap();

        assert_eq!(bytes, verified);
        assert_eq!(cached, verified);
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failed_repair_does_not_replace_an_existing_corrupt_entry() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let file = descriptor(b"expected bytes");
        cache
            .store(
                "mnist",
                &id,
                &file.rel_path,
                &mut Cursor::new(b"old corrupt bytes"),
            )
            .unwrap();
        let (transfer, _) = fake_transfer(b"new corrupt bytes");
        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer,
        );

        let mut reader = source.open("weights.bin").unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        drop(reader);

        assert_eq!(bytes, b"new corrupt bytes");
        let mut cached = cache
            .open("mnist", &id, &file.rel_path)
            .unwrap()
            .expect("the previous entry remains until a verified repair replaces it");
        let mut cached_bytes = Vec::new();
        cached.read_to_end(&mut cached_bytes).unwrap();
        assert_eq!(cached_bytes, b"old corrupt bytes");
    }

    #[test]
    fn dropped_transfer_does_not_publish_a_partial_cache_entry() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let bytes = b"more than four bytes";
        let file = descriptor(bytes);
        let (transfer, fetches) = fake_transfer(bytes);
        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer.clone(),
        );

        let mut reader = source.open("weights.bin").unwrap();
        let mut prefix = [0_u8; 4];
        reader.read_exact(&mut prefix).unwrap();
        drop(reader);

        assert!(cache.open("mnist", &id, &file.rel_path).unwrap().is_none());
        let second = cache.file_source("mnist", &id, file, "mock://weights".to_string(), transfer);
        let _reader = second.open("weights.bin").unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn full_final_chunk_is_not_published_until_eof_is_observed() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let bytes = b"one final chunk";
        let file = descriptor(bytes);
        let (transfer, _) = fake_transfer(bytes);
        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer,
        );

        let mut reader = source.open("weights.bin").unwrap();
        let mut received = vec![0_u8; bytes.len()];
        reader.read_exact(&mut received).unwrap();
        drop(reader);

        assert_eq!(received, bytes);
        assert!(cache.open("mnist", &id, "weights.bin").unwrap().is_none());
    }

    #[test]
    fn cache_uses_the_capability_canonical_path() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let bytes = b"canonical bytes";
        let mut file = descriptor(bytes);
        file.rel_path = ".\\weights.bin".to_string();
        let (transfer, _) = fake_transfer(bytes);
        let source = cache.file_source("mnist", &id, file, "mock://weights".to_string(), transfer);

        let mut reader = source.open("weights.bin").unwrap();
        let mut received = Vec::new();
        reader.read_to_end(&mut received).unwrap();

        assert_eq!(received, bytes);
        assert!(cache.open("mnist", &id, "weights.bin").unwrap().is_some());
    }

    #[test]
    fn unverified_authoritative_bytes_do_not_enter_the_cache() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let id = VersionId::new("opaque-version-id");
        let file = descriptor(b"expected");
        let (transfer, _) = fake_transfer(b"different");
        let source = cache.file_source(
            "mnist",
            &id,
            file.clone(),
            "mock://weights".to_string(),
            transfer,
        );

        let mut reader = source.open("weights.bin").unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();

        assert_eq!(bytes, b"different");
        assert!(cache.open("mnist", &id, &file.rel_path).unwrap().is_none());
    }
}
