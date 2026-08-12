use std::fmt;
use std::io::Read;
use std::sync::Arc;

use crate::{Model, ModelVersion, ModelsError, Page, VersionFile, VersionId};

/// A readable stream returned by a model backend.
pub type VersionFileReader = Box<dyn Read + Send>;

type OpenFile = dyn Fn() -> Result<VersionFileReader, ModelsError> + Send + Sync + 'static;
type OpenCachedFile =
    dyn Fn(&str) -> Result<Option<VersionFileReader>, ModelsError> + Send + Sync + 'static;
type StoreVerifiedFile =
    dyn Fn(&str, &mut dyn Read) -> Result<(), ModelsError> + Send + Sync + 'static;
type InvalidateCachedFile = dyn Fn(&str) -> Result<(), ModelsError> + Send + Sync + 'static;

#[derive(Clone)]
struct FileCacheOps {
    open: Arc<OpenCachedFile>,
    store: Arc<StoreVerifiedFile>,
    invalidate: Arc<InvalidateCachedFile>,
}

/// One backend-provided model file and the primitive used to read its bytes.
///
/// Presigned URLs, authentication, wire protocols, and cache locations stay captured by the
/// callbacks. The [`crate::Models`] capability decides when bytes are verified and when a cache
/// may receive them.
#[derive(Clone)]
pub struct VersionFileSource {
    file: VersionFile,
    open: Arc<OpenFile>,
    cache: Option<FileCacheOps>,
}

impl VersionFileSource {
    /// Creates a source from its published descriptor and a repeatable reader factory.
    pub fn new<F>(file: VersionFile, open: F) -> Self
    where
        F: Fn() -> Result<VersionFileReader, ModelsError> + Send + Sync + 'static,
    {
        Self {
            file,
            open: Arc::new(open),
            cache: None,
        }
    }

    /// Adds backend-scoped cache primitives to this source.
    ///
    /// Each callback receives the capability-validated relative path. `open_cached` returns `None`
    /// on a cache miss. `store_verified` is called only after the complete fetched file set has
    /// passed verification. `invalidate` removes a cached file that fails reading or verification
    /// so the capability can retry it from the authoritative source.
    pub fn with_cache<C, S, I>(mut self, open_cached: C, store_verified: S, invalidate: I) -> Self
    where
        C: Fn(&str) -> Result<Option<VersionFileReader>, ModelsError> + Send + Sync + 'static,
        S: Fn(&str, &mut dyn Read) -> Result<(), ModelsError> + Send + Sync + 'static,
        I: Fn(&str) -> Result<(), ModelsError> + Send + Sync + 'static,
    {
        self.cache = Some(FileCacheOps {
            open: Arc::new(open_cached),
            store: Arc::new(store_verified),
            invalidate: Arc::new(invalidate),
        });
        self
    }

    /// Returns the published path, size, and checksum for this source.
    pub fn file(&self) -> &VersionFile {
        &self.file
    }

    pub(crate) fn open_authoritative(&self) -> Result<VersionFileReader, ModelsError> {
        (self.open)()
    }

    pub(crate) fn open_cached(
        &self,
        rel_path: &str,
    ) -> Result<Option<VersionFileReader>, ModelsError> {
        match &self.cache {
            Some(cache) => (cache.open)(rel_path),
            None => Ok(None),
        }
    }

    pub(crate) fn store_verified(
        &self,
        rel_path: &str,
        reader: &mut dyn Read,
    ) -> Result<(), ModelsError> {
        match &self.cache {
            Some(cache) => (cache.store)(rel_path, reader),
            None => Ok(()),
        }
    }

    pub(crate) fn invalidate_cached(&self, rel_path: &str) -> Result<(), ModelsError> {
        match &self.cache {
            Some(cache) => (cache.invalidate)(rel_path),
            None => Ok(()),
        }
    }

    pub(crate) fn has_cache(&self) -> bool {
        self.cache.is_some()
    }
}

impl fmt::Debug for VersionFileSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VersionFileSource")
            .field("file", &self.file)
            .field("has_cache", &self.cache.is_some())
            .finish_non_exhaustive()
    }
}

/// Backend primitives required by the model capability.
///
/// Implementations are already scoped to their backend location. Consequently, this interface
/// contains no project, namespace, owner, wire, HTTP, or filesystem-cache concepts.
pub trait ModelOps: Send + Sync + 'static {
    /// Lists models in the implementation's scope.
    fn list_models(&self) -> Result<Page<Model>, ModelsError>;

    /// Fetches one model by name.
    fn get_model(&self, name: &str) -> Result<Model, ModelsError>;

    /// Lists published versions of a model.
    fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError>;

    /// Fetches one version using its opaque identity.
    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError>;

    /// Fetches descriptors and reader factories for one version's files.
    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<VersionFileSource>, ModelsError>;
}
