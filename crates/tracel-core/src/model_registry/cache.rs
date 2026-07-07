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
