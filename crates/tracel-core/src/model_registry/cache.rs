//! Local, on-disk cache of downloaded model bundles.
//!
//! Scope is intentionally limited to existence checks and populating the cache on a
//! miss

use std::path::PathBuf;

use tracel_artifact::FileTransferClient;
use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::{
    ArtifactDownloadFile, DownloadError, download_artifacts_to_sink_with_client,
};

use crate::model_registry::ModelRegistryError;

/// Resolves the base cache directory for downloaded models, falling back to the
/// platform's generic cache dir if a `com.tracel.burncentral`-scoped one isn't
/// available (e.g. missing `$HOME` in a stripped-down container). Returns `None` if
/// neither can be determined.
pub(crate) fn resolve_cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "tracel", "burncentral")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .or_else(|| directories::BaseDirs::new().map(|dirs| dirs.cache_dir().join("tracel")))
}

#[derive(Debug, Clone)]
pub(crate) struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn version_dir(&self, name: &str, version: u32) -> PathBuf {
        self.root.join(name).join(version.to_string())
    }

    /// Returns a bundle backed by the cached files if every expected file is already
    /// present on disk, `None` otherwise.
    pub(crate) fn get(
        &self,
        name: &str,
        version: u32,
        files: &[ArtifactDownloadFile],
    ) -> Option<FsBundle> {
        let dir = self.version_dir(name, version);
        let all_present = files.iter().all(|f| dir.join(&f.rel_path).is_file());
        if !all_present {
            return None;
        }

        let rel_paths = files.iter().map(|f| f.rel_path.clone()).collect();
        FsBundle::with_files(dir, rel_paths).ok()
    }

    /// Reserves the destination directory for `name`/`version` as a writable bundle to
    /// download into, so the files land where future lookups will find them.
    pub(crate) fn reserve(&self, name: &str, version: u32) -> Result<FsBundle, std::io::Error> {
        FsBundle::create(self.version_dir(name, version))
    }

    /// Returns the cached bundle for `name`/`version` if all `files` are already present,
    /// otherwise downloads them with `transfer_client` into a freshly reserved directory
    /// and returns the resulting bundle.
    pub(crate) fn get_or_download<FTC: FileTransferClient>(
        &self,
        transfer_client: &FTC,
        name: &str,
        version: u32,
        files: &[ArtifactDownloadFile],
    ) -> Result<FsBundle, ModelRegistryError> {
        if let Some(cached) = self.get(name, version, files) {
            return Ok(cached);
        }

        let mut bundle = self.reserve(name, version).map_err(|e| {
            ModelRegistryError::Download(Box::new(DownloadError::TargetError(e.to_string())))
        })?;
        download_artifacts_to_sink_with_client(transfer_client, &mut bundle, files)
            .map_err(|e| ModelRegistryError::Download(Box::new(e)))?;

        Ok(bundle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Cursor, Read};
    use tracel_artifact::TransferError;
    use tracel_artifact::bundle::{BundleSink, BundleSource};

    fn mock_file(rel_path: &str) -> ArtifactDownloadFile {
        ArtifactDownloadFile {
            rel_path: rel_path.to_string(),
            url: format!("mock://{rel_path}"),
            size_bytes: None,
            checksum: None,
        }
    }

    #[derive(Clone)]
    struct FakeTransferClient {
        files: HashMap<String, Vec<u8>>,
    }

    impl FileTransferClient for FakeTransferClient {
        fn put_reader<R: Read + Send + 'static>(
            &self,
            _url: &str,
            _reader: R,
            _size_bytes: u64,
        ) -> Result<(), TransferError> {
            unimplemented!("model downloads never upload")
        }

        fn get_reader(&self, url: &str) -> Result<Box<dyn Read + Send>, TransferError> {
            let bytes = self
                .files
                .get(url)
                .ok_or_else(|| TransferError::Transport(format!("missing url in mock: {url}")))?;
            Ok(Box::new(Cursor::new(bytes.clone())))
        }
    }

    #[test]
    fn given_empty_cache_when_get_then_returns_none() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let files = vec![mock_file("weights.bin")];

        assert!(cache.get("mnist", 1, &files).is_none());
    }

    #[test]
    fn given_some_absent_files_when_get_then_returns_none() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let mut bundle = cache.reserve("mnist", 1).unwrap();
        bundle
            .put_file("weights.bin", &mut Cursor::new(b"weights"))
            .unwrap();
        drop(bundle);
        let files = vec![mock_file("weights.bin"), mock_file("config.json")];

        assert!(cache.get("mnist", 1, &files).is_none());
    }

    #[test]
    fn given_all_present_files_when_get_then_returns_bundle() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let mut bundle = cache.reserve("mnist", 1).unwrap();
        bundle
            .put_file("weights.bin", &mut Cursor::new(b"weights"))
            .unwrap();
        bundle
            .put_file("config.json", &mut Cursor::new(b"{}"))
            .unwrap();
        drop(bundle);
        let files = vec![mock_file("weights.bin"), mock_file("config.json")];

        let cached = cache.get("mnist", 1, &files).expect("expected a cache hit");

        let mut contents = String::new();
        cached
            .open("weights.bin")
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "weights");
    }

    #[test]
    fn given_wrong_parameter_when_get_then_returns_none() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let mut bundle = cache.reserve("mnist", 1).unwrap();
        bundle
            .put_file("weights.bin", &mut Cursor::new(b"weights"))
            .unwrap();
        drop(bundle);
        let files = vec![mock_file("weights.bin")];

        assert!(cache.get("mnist", 2, &files).is_none());
        assert!(cache.get("resnet", 1, &files).is_none());
    }

    #[test]
    fn when_reserve_then_new_directory_is_created() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());

        let bundle = cache.reserve("mnist", 1).unwrap();

        assert!(bundle.root().is_dir());
        assert_eq!(bundle.root(), root.path().join("mnist").join("1"));
    }

    #[test]
    fn given_cache_hit_when_get_or_download_then_returns_cached_bundle_without_transfer() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let mut bundle = cache.reserve("mnist", 1).unwrap();
        bundle
            .put_file("weights.bin", &mut Cursor::new(b"weights"))
            .unwrap();
        drop(bundle);
        let files = vec![mock_file("weights.bin")];
        let transfer_client = FakeTransferClient {
            files: HashMap::new(),
        };

        let bundle = cache
            .get_or_download(&transfer_client, "mnist", 1, &files)
            .expect("expected cache hit");

        let mut contents = String::new();
        bundle
            .open("weights.bin")
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "weights");
    }

    #[test]
    fn given_cache_miss_when_get_or_download_then_downloads_and_returns_bundle() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let files = vec![mock_file("weights.bin")];
        let transfer_client = FakeTransferClient {
            files: HashMap::from([("mock://weights.bin".to_string(), b"weights".to_vec())]),
        };

        let bundle = cache
            .get_or_download(&transfer_client, "mnist", 1, &files)
            .expect("expected download to succeed");

        let mut contents = String::new();
        bundle
            .open("weights.bin")
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "weights");
    }

    #[test]
    fn given_transfer_error_when_get_or_download_then_returns_download_error() {
        let root = tempfile::tempdir().unwrap();
        let cache = ModelCache::new(root.path().to_path_buf());
        let files = vec![mock_file("weights.bin")];
        let transfer_client = FakeTransferClient {
            files: HashMap::new(),
        };

        let result = cache.get_or_download(&transfer_client, "mnist", 1, &files);

        match result {
            Err(ModelRegistryError::Download(e)) => {
                let e = e
                    .downcast_ref::<DownloadError>()
                    .expect("expected DownloadError");
                assert!(matches!(e, DownloadError::Transfer { .. }));
            }
            other => panic!("expected Download error, got {other:?}"),
        }
    }
}
