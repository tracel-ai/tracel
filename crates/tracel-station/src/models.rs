use std::sync::Arc;

use bytes::Bytes;
use tracel_artifact::{HttpTransferClient, TransferClient, TransferError};
use tracel_client::station::model::request::CreateModelRequest;
use tracel_client::station::model::response::{
    ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
    ModelVersionResponse,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionManifest, VersionSpec,
};
use tracel_task::{Spawn, Streaming, Task};

use crate::StationError;
use crate::station::StationInner;
use crate::wire::station_timestamp;

#[derive(Clone)]
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

impl StationModelOps {
    fn spawn(&self) -> &dyn Spawn {
        &*self.station.spawn
    }
}

impl ModelOps for StationModelOps {
    fn list_models(&self) -> Task<Vec<Model>, ModelsError> {
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            this.station
                .client
                .models()
                .list()
                .await
                .map(models_from_wire)
                .map_err(station_failure)
        })
    }

    fn get_model(&self, name: String) -> Task<Model, ModelsError> {
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            this.station
                .client
                .models()
                .get(&name)
                .await
                .map(model_from_wire)
                .map_err(|error| map_model_error(error, &name))
        })
    }

    fn list_versions(&self, model: String) -> Task<Vec<ModelVersion>, ModelsError> {
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            this.station
                .client
                .models()
                .versions(&model)
                .await
                .map(model_versions_from_wire)
                .map_err(|error| map_model_error(error, &model))
        })
    }

    fn get_version(&self, model: String, spec: VersionSpec) -> Task<ModelVersion, ModelsError> {
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            let id = match &spec {
                VersionSpec::Exact(id) => id.clone(),
                // The Station has no latest-version route, so the listing answers it.
                VersionSpec::Latest => {
                    return this
                        .list_versions(model.clone())
                        .await?
                        .into_iter()
                        .max_by_key(|version| version.version)
                        .ok_or(ModelsError::VersionNotFound {
                            model,
                            version: spec,
                        });
                }
            };

            let route = this.route_version(&model, &id)?;
            this.station
                .client
                .models()
                .version(&model, route)
                .await
                .map(model_version_from_wire)
                .map_err(|error| map_version_error(error, &model, &id))
        })
    }

    fn fetch_version_files(
        &self,
        model: String,
        id: VersionId,
    ) -> Task<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = match self.route_version(&model, &id) {
            Ok(route) => route,
            Err(error) => return Task::failed(error),
        };
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            let station = &this.station;
            station
                .client
                .models()
                .download(&model, route)
                .await
                .map_err(|error| map_version_error(error, &model, &id))
                .map(|response| file_sources_from_wire(&station.transfer, &station.spawn, response))
        })
    }

    fn create_model(&self, name: String, description: Option<String>) -> Task<Model, ModelsError> {
        let this = self.clone();
        Task::spawn(self.spawn(), async move {
            this.station
                .client
                .models()
                .create(CreateModelRequest { name, description })
                .await
                .map(model_from_wire)
                .map_err(station_failure)
        })
    }

    fn publish_version(
        &self,
        _model: String,
        _files: Vec<VersionFile>,
        _contents: Arc<dyn tracel_artifact::upload::MultipartUploadSource>,
        _metadata: Option<serde_json::Value>,
        _observer: Box<dyn tracel_artifact::TransferObserver>,
    ) -> Task<ModelVersion, ModelsError> {
        Task::failed(ModelsError::other(
            "publishing a model version is not implemented for the station yet",
        ))
    }
}

struct StationVersionFileSource {
    file: VersionFile,
    url: String,
    transfer: HttpTransferClient,
    spawn: Arc<dyn Spawn>,
}

impl VersionFileSource for StationVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: String) -> Streaming<Bytes, TransferError> {
        let transfer = self.transfer.clone();
        let url = self.url.clone();
        let size = self.file.size_bytes;
        Streaming::spawn(&*self.spawn, 1, |sink| async move {
            match transfer.get(&url, Some(size)).await {
                Ok(body) => sink.forward(body).await,
                Err(error) => sink.fail(error).await,
            }
        })
    }
}

fn file_sources_from_wire(
    transfer: &HttpTransferClient,
    spawn: &Arc<dyn Spawn>,
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
                transfer: transfer.clone(),
                spawn: Arc::clone(spawn),
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
        // The Station's version response carries no metadata.
        metadata: serde_json::Value::Null,
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
    }
}

fn station_failure(error: tracel_client::ClientError) -> ModelsError {
    match StationError::from(error) {
        StationError::Transport(reason) => ModelsError::Transport(reason),
        error => ModelsError::other(error),
    }
}

fn map_model_error(error: tracel_client::ClientError, name: &str) -> ModelsError {
    if error.is_not_found() {
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
    if error.is_not_found() {
        return ModelsError::VersionNotFound {
            model: model.to_string(),
            version: VersionSpec::Exact(id.clone()),
        };
    }
    station_failure(error)
}
