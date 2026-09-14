use tracel_artifact::upload::MultipartUploadSource;
use tracel_artifact::{ByteStream, TransferObserver};
use tracel_task::DynFuture;

use crate::{Model, ModelVersion, ModelsError, VersionFile, VersionId, VersionSpec};

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
    fn open<'a>(
        &'a self,
        canonical_path: &'a str,
    ) -> DynFuture<'a, Result<ByteStream, ModelsError>>;
}

/// Backend primitives required by the model capability.
///
/// An implementation is already scoped to one location, so it is never asked which one.
///
/// Implementations should use the dedicated not-found variants for missing models and versions,
/// and [`ModelsError::Transport`] for communication failures. Backend-specific failures may be
/// preserved with [`ModelsError::other`].
pub trait ModelOps: Send + Sync + 'static {
    /// Lists models in the implementation's scope.
    fn list_models(&self) -> DynFuture<'_, Result<Vec<Model>, ModelsError>>;

    /// Fetches one model by name.
    fn get_model<'a>(&'a self, name: &'a str) -> DynFuture<'a, Result<Model, ModelsError>>;

    /// Lists published versions of a model.
    fn list_versions<'a>(
        &'a self,
        model: &'a str,
    ) -> DynFuture<'a, Result<Vec<ModelVersion>, ModelsError>>;

    /// Resolves a version selector against a model.
    fn get_version<'a>(
        &'a self,
        model: &'a str,
        spec: VersionSpec,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>>;

    /// Fetches the backend-owned file sources for one version.
    fn fetch_version_files<'a>(
        &'a self,
        model: &'a str,
        id: &'a VersionId,
    ) -> DynFuture<'a, Result<Vec<Box<dyn VersionFileSource>>, ModelsError>>;

    /// Creates a model that can hold versions.
    fn create_model<'a>(
        &'a self,
        name: &'a str,
        description: Option<&'a str>,
    ) -> DynFuture<'a, Result<Model, ModelsError>>;

    /// Publishes a version of `model` containing the files the capability measured.
    ///
    /// The bytes are read from `contents` by each file's relative path. Whether they travel in
    /// one request or a hundred, and whether the version appears atomically or is assembled
    /// first, is the implementation's business: a version either becomes visible or it does not.
    fn publish_version<'a>(
        &'a self,
        model: &'a str,
        files: &'a [VersionFile],
        contents: &'a dyn MultipartUploadSource,
        metadata: Option<&'a serde_json::Value>,
        observer: &'a mut dyn TransferObserver,
    ) -> DynFuture<'a, Result<ModelVersion, ModelsError>>;
}
