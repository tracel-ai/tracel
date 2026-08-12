use std::sync::Arc;

use tracel_artifact::ReqwestTransferClient;
use tracel_client::{ClientError, StationClient};
use tracel_models::{
    Model, ModelOps, ModelVersion, Models, ModelsError, Page, VersionFile, VersionFileSource,
    VersionId, VersionManifest,
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
    model_version_routes: crate::model_routes::ModelVersionRoutes,
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
            model_version_routes: crate::model_routes::ModelVersionRoutes::default(),
        })
    }

    /// Returns model operations scoped to this Station.
    pub fn models(&self) -> Models {
        Models::new(Arc::new(self.clone()))
    }

    fn resolve_model_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        if let Some(version) = self.model_version_routes.get(model, id)? {
            return Ok(version);
        }

        let response = self
            .client
            .models()
            .versions(model)
            .map_err(|error| model_request_error(error, model))?;

        for version in response.items {
            self.model_version_routes.remember(
                model,
                VersionId::new(version.id),
                version.version,
            )?;
        }
        self.model_version_routes
            .get(model, id)?
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
    ) -> Box<dyn VersionFileSource> {
        let file = VersionFile {
            rel_path: response.rel_path,
            size_bytes: response.size_bytes,
            checksum: response.checksum,
        };
        Box::new(self.model_cache.file_source(
            model,
            id,
            file,
            response.url,
            self.file_transfer_client.clone(),
        ))
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
        let response = self
            .client
            .models()
            .versions(model)
            .map_err(|error| model_request_error(error, model))?;
        for version in &response.items {
            self.model_version_routes.remember(
                model,
                VersionId::new(&version.id),
                version.version,
            )?;
        }
        Ok(Page {
            items: response.items.into_iter().map(map_version).collect(),
            total: response.total,
        })
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
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
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
        published_by: None,
        created_at: response.created_at,
        version_count: response.version_count,
        latest_version: None,
    }
}

fn map_version(response: tracel_client::station::model::ModelVersionResponse) -> ModelVersion {
    ModelVersion {
        id: VersionId::new(response.id),
        version: response.version,
        size_bytes: response.size,
        checksum: response.checksum,
        published_by: None,
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
    if client_error_is_not_visible(&error) {
        ModelsError::ModelNotFound {
            name: name.to_string(),
        }
    } else {
        map_client_error(error)
    }
}

fn scope_request_error(error: ClientError) -> ModelsError {
    if client_error_is_not_visible(&error) {
        ModelsError::ScopeNotFound
    } else {
        map_client_error(error)
    }
}

fn version_request_error(error: ClientError, model: &str, id: &VersionId) -> ModelsError {
    if client_error_is_not_visible(&error) {
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
        ClientError::Unauthorized => ModelsError::SessionExpired,
        ClientError::ApiError { ref status, .. } if status.as_u16() == 401 => {
            ModelsError::SessionExpired
        }
        ClientError::BadSessionId => {
            ModelsError::InvalidResponse("login response omitted the session cookie".to_string())
        }
        ClientError::Serialization(error) => ModelsError::InvalidResponse(error.to_string()),
        error => ModelsError::Transport(error.to_string()),
    }
}

fn client_error_is_not_visible(error: &ClientError) -> bool {
    error.is_not_found()
        || matches!(
            error,
            ClientError::ApiError { status, .. }
                if status.as_u16() == 403 || status.as_u16() == 404
        )
}
