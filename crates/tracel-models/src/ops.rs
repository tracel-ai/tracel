use std::sync::Arc;

use bytes::Bytes;
use tracel_artifact::upload::MultipartUploadSource;
use tracel_artifact::{TransferError, TransferObserver};
use tracel_task::{Job, Streaming};

use crate::{Model, ModelVersion, ModelsError, VersionFile, VersionId, VersionSpec};

/// One backend-owned file in a model version.
///
/// Implementations own transport, authentication, presigning, and any backend-specific cache
/// behavior. [`crate::Models`] owns descriptor validation, transfer orchestration, staging,
/// integrity verification, progress, and delivery.
pub trait VersionFileSource: Send + Sync + 'static {
    /// Returns the published descriptor that the capability must verify.
    fn file(&self) -> &VersionFile;

    /// Starts reading the file from byte zero using its capability-validated logical path.
    ///
    /// The supplied path is the canonical form of [`Self::file`]'s published relative path. It
    /// lets implementations use one stable identity for backend-private concerns without taking
    /// ownership of path validation. A failure to open is the stream's first item.
    fn open(&self, canonical_path: String) -> Streaming<Bytes, TransferError>;
}

/// Backend primitives required by the model capability.
///
/// An implementation is already scoped to one location, so it is never asked which one. Every
/// operation is handed back as a [`Job`] that any executor can drive: an implementation whose
/// transport needs a runtime attaches the work to the one it owns before handing it back.
///
/// Implementations should use the dedicated not-found variants for missing models and versions,
/// and [`ModelsError::Transport`] for communication failures. Backend-specific failures may be
/// preserved with [`ModelsError::other`].
pub trait ModelOps: Send + Sync + 'static {
    /// Lists models in the implementation's scope.
    fn list_models(&self) -> Job<Vec<Model>, ModelsError>;

    /// Fetches one model by name.
    fn get_model(&self, name: String) -> Job<Model, ModelsError>;

    /// Lists published versions of a model.
    fn list_versions(&self, model: String) -> Job<Vec<ModelVersion>, ModelsError>;

    /// Resolves a version selector against a model.
    fn get_version(&self, model: String, spec: VersionSpec) -> Job<ModelVersion, ModelsError>;

    /// Fetches the backend-owned file sources for one version.
    fn fetch_version_files(
        &self,
        model: String,
        id: VersionId,
    ) -> Job<Vec<Box<dyn VersionFileSource>>, ModelsError>;

    /// Creates a model that can hold versions.
    fn create_model(&self, name: String, description: Option<String>) -> Job<Model, ModelsError>;

    /// Publishes a version of `model` containing the files the capability measured.
    ///
    /// The bytes are read from `contents` by each file's relative path. Whether they travel in
    /// one request or a hundred, and whether the version appears atomically or is assembled
    /// first, is the implementation's business: a version either becomes visible or it does not.
    fn publish_version(
        &self,
        model: String,
        files: Vec<VersionFile>,
        contents: Arc<dyn MultipartUploadSource>,
        metadata: Option<serde_json::Value>,
        observer: Box<dyn TransferObserver>,
    ) -> Job<ModelVersion, ModelsError>;
}
