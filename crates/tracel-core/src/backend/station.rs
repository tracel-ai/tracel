use std::sync::Arc;

use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use tracel_client::{ClientError, StationClient};
use tracel_models::{
    ExperimentSource, Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile,
    VersionFileSource, VersionId, VersionManifest,
};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum StationError {
    /// No platform cache directory could be resolved for model downloads.
    #[error("could not determine a cache directory for downloaded models")]
    NoCacheDir,
}

#[derive(Clone)]
pub struct StationBackend {
    pub(crate) client: StationClient,
    pub(crate) file_transfer_client: ReqwestTransferClient,
    pub(crate) model_cache: crate::model_cache::ModelCache,
}

impl StationBackend {
    /// Creates a backend for the Station at `url`.
    pub fn new(url: Url) -> Result<StationBackend, StationError> {
        let cache_root = crate::resolve_cache_dir()
            .ok_or(StationError::NoCacheDir)?
            .join("station")
            .join(crate::model_cache::opaque_cache_key(url.as_str()))
            .join("models");

        Ok(StationBackend {
            client: StationClient::from_url(url),
            file_transfer_client: ReqwestTransferClient::new(),
            model_cache: crate::model_cache::ModelCache::new(cache_root),
        })
    }

    /// Returns model operations scoped to this Station.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(self.clone()))
    }

    fn resolve_model_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        let response = self
            .client
            .models()
            .versions(model)
            .map_err(|error| model_request_error(error, model))?;

        response
            .items
            .into_iter()
            .find_map(|version| (version.id == id.as_str()).then_some(version.version))
            .ok_or_else(|| ModelsError::VersionNotFound {
                model: model.to_string(),
                id: id.clone(),
            })
    }

    fn version_file_source(
        &self,
        model: &str,
        id: &VersionId,
        response: tracel_client::station::model::PresignedModelFileUrlResponse,
    ) -> VersionFileSource {
        let rel_path = response.rel_path;
        let file = VersionFile {
            rel_path: rel_path.clone(),
            size_bytes: response.size_bytes,
            checksum: response.checksum,
        };

        let transfer_client = self.file_transfer_client.clone();
        let url = response.url;

        let open_cache = self.model_cache.clone();
        let open_model = model.to_string();
        let open_id = id.clone();
        let store_cache = self.model_cache.clone();
        let store_model = model.to_string();
        let store_id = id.clone();
        let invalidate_cache = self.model_cache.clone();
        let invalidate_model = model.to_string();
        let invalidate_id = id.clone();

        VersionFileSource::new(file, move || {
            transfer_client
                .get_reader(&url)
                .map_err(|error| ModelsError::Transport(error.to_string()))
        })
        .with_cache(
            move |path| open_cache.open(&open_model, &open_id, path),
            move |path, reader| store_cache.store(&store_model, &store_id, path, reader),
            move |path| invalidate_cache.invalidate(&invalidate_model, &invalidate_id, path),
        )
    }
}

impl ModelOps for StationBackend {
    fn list_models(&self) -> Result<Page<Model>, ModelsError> {
        self.client
            .models()
            .list()
            .map(|response| Page {
                items: response.items.into_iter().map(map_model).collect(),
                total: response.total,
            })
            .map_err(scope_request_error)
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.client
            .models()
            .get(name)
            .map(map_model)
            .map_err(|error| model_request_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Page<ModelVersion>, ModelsError> {
        self.client
            .models()
            .versions(model)
            .map(|response| Page {
                items: response.items.into_iter().map(map_version).collect(),
                total: response.total,
            })
            .map_err(|error| model_request_error(error, model))
    }

    fn get_version(&self, model: &str, id: &VersionId) -> Result<ModelVersion, ModelsError> {
        let version = self.resolve_model_version(model, id)?;
        self.client
            .models()
            .version(model, version)
            .map(map_version)
            .map_err(|error| version_request_error(error, model, id))
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<VersionFileSource>, ModelsError> {
        let version = self.resolve_model_version(model, id)?;
        self.client
            .models()
            .download(model, version)
            .map_err(|error| version_request_error(error, model, id))
            .map(|response| {
                response
                    .files
                    .into_iter()
                    .map(|file| self.version_file_source(model, id, file))
                    .collect()
            })
    }
}

fn map_model(response: tracel_client::station::model::ModelResponse) -> Model {
    Model {
        id: response.id,
        name: response.name,
        description: response.description,
        created_at: response.created_at,
        version_count: response.version_count,
        latest_version: None,
    }
}

fn map_version(response: tracel_client::station::model::ModelVersionResponse) -> ModelVersion {
    ModelVersion {
        id: VersionId::new(response.id),
        experiment: response.experiment.map(|source| ExperimentSource {
            id: source.id,
            experiment_num: source.experiment_num,
        }),
        version: response.version,
        size_bytes: response.size,
        checksum: response.checksum,
        created_at: response.created_at,
        manifest: VersionManifest {
            files: response
                .manifest
                .files
                .into_iter()
                .map(|file| VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                })
                .collect(),
        },
        metadata: serde_json::Value::Null,
    }
}

fn model_request_error(error: ClientError, name: &str) -> ModelsError {
    if error.is_not_found() {
        ModelsError::ModelNotFound {
            name: name.to_string(),
        }
    } else {
        map_client_error(error)
    }
}

fn scope_request_error(error: ClientError) -> ModelsError {
    if error.is_not_found() {
        ModelsError::ScopeNotFound
    } else {
        map_client_error(error)
    }
}

fn version_request_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if error.is_not_found() {
        ModelsError::VersionNotFound {
            model: model.to_string(),
            id: id.clone(),
        }
    } else {
        map_client_error(error)
    }
}

fn map_client_error(error: ClientError) -> ModelsError {
    match error {
        ClientError::Unauthorized | ClientError::BadSessionId => {
            ModelsError::Authentication(error.to_string())
        }
        ClientError::ApiError { ref status, .. }
            if status.as_u16() == 401 || status.as_u16() == 403 =>
        {
            ModelsError::Authentication(error.to_string())
        }
        ClientError::Serialization(error) => ModelsError::InvalidResponse(error.to_string()),
        error => ModelsError::Transport(error.to_string()),
    }
}
