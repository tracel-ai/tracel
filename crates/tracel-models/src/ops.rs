use std::io::Read;

use tracel_artifact::TransferObserver;
use tracel_artifact::upload::MultipartUploadSource;

use crate::{Model, ModelVersion, ModelsError, VersionFile, VersionId, VersionSpec};

/// A readable stream for one model-version file.
pub type VersionFileReader = Box<dyn Read + Send>;

/// One backend-owned file in a model version.
///
/// Implementations own transport, authentication, presigning, and any backend-specific cache
/// behavior. [`crate::Models`] owns descriptor validation, transfer orchestration, staging,
/// integrity verification, progress, and delivery.
pub trait VersionFileSource: Send + Sync + 'static {
    /// Returns the published descriptor that the capability must verify.
    fn file(&self) -> &VersionFile;

    /// Opens the file at byte zero using its capability-validated logical path.
    ///
    /// The supplied path is the canonical form of [`Self::file`]'s published relative path. It
    /// lets implementations use one stable identity for backend-private concerns without taking
    /// ownership of path validation.
    fn open(&self, canonical_path: &str) -> Result<VersionFileReader, ModelsError>;
}

/// Backend primitives required by the model capability.
///
/// An implementation is already scoped to one location, so it is never asked which one.
///
/// Report a missing model or version as such; report everything else through
/// [`ModelsError::other`], which keeps the implementation's own error type intact.
pub trait ModelOps: Send + Sync + 'static {
    /// Lists models in the implementation's scope.
    fn list_models(&self) -> Result<Vec<Model>, ModelsError>;

    /// Fetches one model by name.
    fn get_model(&self, name: &str) -> Result<Model, ModelsError>;

    /// Lists published versions of a model.
    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError>;

    /// Resolves a version selector against a model.
    fn get_version(&self, model: &str, spec: VersionSpec) -> Result<ModelVersion, ModelsError>;

    /// Fetches the backend-owned file sources for one version.
    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError>;

    /// Creates a model that can hold versions.
    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError>;

    /// Publishes a version of `model` containing the files the capability measured.
    ///
    /// The bytes are read from `contents` by each file's relative path. Whether they travel in
    /// one request or a hundred, and whether the version appears atomically or is assembled
    /// first, is the implementation's business: a version either becomes visible or it does not.
    fn publish_version(
        &self,
        model: &str,
        files: &[VersionFile],
        contents: &dyn MultipartUploadSource,
        metadata: Option<&serde_json::Value>,
        observer: &mut dyn TransferObserver,
    ) -> Result<ModelVersion, ModelsError>;
}
