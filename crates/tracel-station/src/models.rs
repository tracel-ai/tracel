use std::sync::Arc;

use tracel_artifact::{FileTransferClient, ReqwestTransferClient};
use tracel_client::station::model::request::CreateModelRequest;
use tracel_client::station::model::response::{
    ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
    ModelVersionResponse,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileReader, VersionFileSource,
    VersionId, VersionManifest, VersionSpec,
};

use crate::StationError;
use crate::station::StationInner;
use crate::wire::station_timestamp;

pub struct StationModelOps {
    pub station: Arc<StationInner>,
}

impl StationModelOps {
    fn route_version(&self, model: &str, id: &VersionId) -> Result<u32, ModelsError> {
        id.as_str()
            .parse()
            .map_err(|_| ModelsError::VersionNotFound {
                model: model.to_string(),
                version: VersionSpec::Exact(id.clone()),
            })
    }
}

impl ModelOps for StationModelOps {
    fn list_models(&self) -> Result<Vec<Model>, ModelsError> {
        let response = self
            .station
            .client
            .models()
            .list()
            .map_err(station_failure)?;
        Ok(models_from_wire(response))
    }

    fn get_model(&self, name: &str) -> Result<Model, ModelsError> {
        self.station
            .client
            .models()
            .get(name)
            .map(model_from_wire)
            .map_err(|error| map_model_error(error, name))
    }

    fn list_versions(&self, model: &str) -> Result<Vec<ModelVersion>, ModelsError> {
        let response = self
            .station
            .client
            .models()
            .versions(model)
            .map_err(|error| map_model_error(error, model))?;
        Ok(model_versions_from_wire(response))
    }

    fn get_version(&self, model: &str, spec: VersionSpec) -> Result<ModelVersion, ModelsError> {
        let id = match &spec {
            VersionSpec::Exact(id) => id.clone(),
            VersionSpec::Latest => self
                .list_versions(model)?
                .into_iter()
                .max_by_key(|version| version.version)
                .map(|version| version.id)
                .ok_or_else(|| ModelsError::VersionNotFound {
                    model: model.to_string(),
                    version: spec.clone(),
                })?,
        };

        let route = self.route_version(model, &id)?;
        self.station
            .client
            .models()
            .version(model, route)
            .map(model_version_from_wire)
            .map_err(|error| map_version_error(error, model, &id))
    }

    fn fetch_version_files(
        &self,
        model: &str,
        id: &VersionId,
    ) -> Result<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = self.route_version(model, id)?;
        let response = self
            .station
            .client
            .models()
            .download(model, route)
            .map_err(|error| map_version_error(error, model, id))?;
        Ok(file_sources_from_wire(
            &self.station.transfer_client,
            response,
        ))
    }

    fn create_model(&self, name: &str, description: Option<&str>) -> Result<Model, ModelsError> {
        self.station
            .client
            .models()
            .create(CreateModelRequest {
                name: name.to_string(),
                description: description.map(str::to_string),
            })
            .map(model_from_wire)
            .map_err(station_failure)
    }

    fn publish_version(
        &self,
        _model: &str,
        _files: &[VersionFile],
        _contents: &dyn tracel_artifact::upload::MultipartUploadSource,
        _metadata: Option<&serde_json::Value>,
        _observer: &mut dyn tracel_artifact::TransferObserver,
    ) -> Result<ModelVersion, ModelsError> {
        Err(ModelsError::other(
            "publishing a model version is not implemented for the station yet",
        ))
    }
}

struct StationVersionFileSource {
    file: VersionFile,
    url: String,
    transfer_client: ReqwestTransferClient,
}

impl VersionFileSource for StationVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: &str) -> Result<VersionFileReader, ModelsError> {
        self.transfer_client
            .get_reader(&self.url)
            .map_err(|error| ModelsError::other(StationError::Transport(error.to_string())))
    }
}

fn file_sources_from_wire(
    transfer_client: &ReqwestTransferClient,
    response: ModelDownloadResponse,
) -> Vec<Box<dyn VersionFileSource>> {
    response
        .files
        .into_iter()
        .map(|file| {
            Box::new(StationVersionFileSource {
                file: VersionFile {
                    rel_path: file.rel_path,
                    size_bytes: file.size_bytes,
                    checksum: file.checksum,
                },
                url: file.url,
                transfer_client: transfer_client.clone(),
            }) as Box<dyn VersionFileSource>
        })
        .collect()
}

fn models_from_wire(response: ModelListResponse) -> Vec<Model> {
    response.items.into_iter().map(model_from_wire).collect()
}

fn model_from_wire(response: ModelResponse) -> Model {
    Model {
        id: response.id,
        name: response.name,
        description: response.description,
        published_by: None,
        created_at: station_timestamp(&response.created_at),
        version_count: response.version_count,
        latest_version: None,
    }
}

fn model_versions_from_wire(response: ModelVersionListResponse) -> Vec<ModelVersion> {
    response
        .items
        .into_iter()
        .map(model_version_from_wire)
        .collect()
}

fn model_version_from_wire(response: ModelVersionResponse) -> ModelVersion {
    ModelVersion {
        id: VersionId::new(response.version.to_string()),
        version: Some(response.version),
        size_bytes: response.size,
        checksum: response.checksum,
        published_by: None,
        created_at: station_timestamp(&response.created_at),
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

fn station_failure(error: tracel_client::ClientError) -> ModelsError {
    ModelsError::other(StationError::from(error))
}

fn map_model_error(error: tracel_client::ClientError, name: &str) -> ModelsError {
    if crate::error::client_error_is_not_found(&error) {
        return ModelsError::ModelNotFound {
            name: name.to_string(),
        };
    }
    station_failure(error)
}

fn map_version_error(
    error: tracel_client::ClientError,
    model: &str,
    id: &VersionId,
) -> ModelsError {
    if crate::error::client_error_is_not_found(&error) {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    station_failure(error)
}
