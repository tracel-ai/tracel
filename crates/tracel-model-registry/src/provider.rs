use std::sync::Arc;

use serde_json::Value;
use tracel_artifact::bundle::BundleSource;

use crate::error::ModelRegistryError;
use crate::model::{Availability, Model, Revision, Version};

/// Convenience alias for registry results.
pub type Result<T> = std::result::Result<T, ModelRegistryError>;

/// A bundle of a model version's files, read by name via [`BundleSource`]. Used symmetrically: as
/// the input to [`publish`](ModelRegistryProvider::publish) and as the materialized output of
/// [`fetch`](ModelRegistryProvider::fetch). Reads are synchronous streams (`open`); when the bundle
/// is file-backed, `local_path` exposes a real file for lazy / zero-copy loading (e.g. burn-pack's
/// `Reader::from_file`).
pub type Artifacts = Box<dyn BundleSource + Send>;

/// Backend port for a model registry, implemented by the local engine, station, or cloud.
///
/// Metadata reads are synchronous because they are served from in-memory state. Fetching
/// artifacts, publishing a version, and anything else that touches storage or the network is
/// asynchronous.
#[async_trait::async_trait]
pub trait ModelRegistryProvider: Send + Sync {
    /// Create a new, empty model. Versions are added with [`publish`](Self::publish).
    async fn create_model(&self, name: &str, description: Option<String>) -> Result<Model>;

    /// List the models in the registry.
    fn models(&self) -> Result<Vec<Model>>;

    /// Get a model by name.
    fn model(&self, name: &str) -> Result<Option<Model>>;

    /// List a model's versions, oldest first.
    fn versions(&self, name: &str) -> Result<Vec<Version>>;

    /// Materialize the files of a model version from the **local** store. A version whose
    /// artifacts are not locally present fails with
    /// [`ModelRegistryError::ArtifactsUnavailable`]. Pulling a remote version's bytes is an
    /// explicit engine operation, never a `fetch` side effect.
    async fn fetch(&self, name: &str, revision: Revision) -> Result<Artifacts>;

    /// Whether a version's artifact bytes are locally present (a probe of the local store,
    /// computed, never stored). `fetch` succeeds iff this is [`Availability::Present`].
    async fn availability(&self, name: &str, revision: Revision) -> Result<Availability>;

    /// Publish a new version of a model from a bundle of files plus opaque metadata.
    async fn publish(&self, name: &str, artifacts: Artifacts, metadata: Value) -> Result<Version>;

    /// Delete a model and all its versions.
    async fn delete(&self, name: &str) -> Result<()>;
}

/// Entry point for working with a model registry against a backend.
#[derive(Clone)]
pub struct ModelRegistryModule {
    provider: Arc<dyn ModelRegistryProvider>,
}

impl ModelRegistryModule {
    /// Create a module backed by the given provider.
    pub fn new(provider: Arc<dyn ModelRegistryProvider>) -> Self {
        Self { provider }
    }

    /// Create a new, empty model. Add versions with [`publish`](Self::publish).
    pub async fn create_model(&self, name: &str, description: Option<String>) -> Result<Model> {
        self.provider.create_model(name, description).await
    }

    /// List the models in the registry.
    pub fn models(&self) -> Result<Vec<Model>> {
        self.provider.models()
    }

    /// Get a model by name.
    pub fn model(&self, name: &str) -> Result<Option<Model>> {
        self.provider.model(name)
    }

    /// List a model's versions.
    pub fn versions(&self, name: &str) -> Result<Vec<Version>> {
        self.provider.versions(name)
    }

    /// Materialize a version's files into a bundle. Read them by name with [`BundleSource::open`],
    /// or load lazily from [`BundleSource::local_path`] when the bundle is file-backed. Fails
    /// with [`ModelRegistryError::ArtifactsUnavailable`] when the version's bytes are not
    /// locally present (see [`availability`](Self::availability)).
    pub async fn fetch(&self, name: &str, revision: impl Into<Revision>) -> Result<Artifacts> {
        self.provider.fetch(name, revision.into()).await
    }

    /// Whether a version's artifact bytes are locally present (a probe of the local store).
    pub async fn availability(
        &self,
        name: &str,
        revision: impl Into<Revision>,
    ) -> Result<Availability> {
        self.provider.availability(name, revision.into()).await
    }

    /// Publish a new version from a bundle of files plus opaque metadata.
    pub async fn publish(
        &self,
        name: &str,
        artifacts: Artifacts,
        metadata: Value,
    ) -> Result<Version> {
        self.provider.publish(name, artifacts, metadata).await
    }

    /// Delete a model and all its versions.
    pub async fn delete(&self, name: &str) -> Result<()> {
        self.provider.delete(name).await
    }
}
