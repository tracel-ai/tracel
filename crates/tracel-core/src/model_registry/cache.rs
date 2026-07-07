//! Local, on-disk cache of downloaded model bundles.
//!
//! Scope is intentionally limited to existence checks: does a cached copy already
//! exist for a given model name/version? Cache eviction is handled elsewhere.

use std::path::PathBuf;

use tracel_artifact::bundle::FsBundle;
use tracel_artifact::download::ArtifactDownloadFile;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};
    use tracel_artifact::bundle::{BundleSink, BundleSource};

    fn mock_file(rel_path: &str) -> ArtifactDownloadFile {
        ArtifactDownloadFile {
            rel_path: rel_path.to_string(),
            url: format!("mock://{rel_path}"),
            size_bytes: None,
            checksum: None,
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
}
