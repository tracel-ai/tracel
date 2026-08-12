//! Backend-owned local cache primitives for verified model files.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::Digest;
use tracel_models::{ModelsError, VersionFileReader, VersionId};

#[derive(Debug, Clone)]
pub(crate) struct ModelCache {
    root: PathBuf,
}

impl ModelCache {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) fn open(
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

    pub(crate) fn store(
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

    pub(crate) fn invalidate(
        &self,
        model: &str,
        id: &VersionId,
        rel_path: &str,
    ) -> Result<(), ModelsError> {
        let path = self.file_path(model, id, rel_path)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ModelsError::Cache(error.to_string())),
        }
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

    use super::*;

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
}
