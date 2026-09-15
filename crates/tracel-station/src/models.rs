use std::sync::Arc;

use bytes::Bytes;
use futures::{TryStreamExt, stream};
use tracel_artifact::{TransferClient, TransferError};
use tracel_client::station::model::request::CreateModelRequest;
use tracel_client::station::model::response::{
    ModelDownloadResponse, ModelListResponse, ModelResponse, ModelVersionListResponse,
    ModelVersionResponse,
};
use tracel_models::{
    Model, ModelOps, ModelVersion, ModelsError, VersionFile, VersionFileSource, VersionId,
    VersionManifest, VersionSpec,
};
use tracel_task::{Job, Streaming};

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

impl ModelOps for StationModelOps {
    fn list_models(&self) -> Job<Vec<Model>, ModelsError> {
        let this = self.clone();
        self.station.attach(async move {
            this.station
                .client
                .models()
                .list()
                .await
                .map(models_from_wire)
                .map_err(station_failure)
        })
    }

    fn get_model(&self, name: String) -> Job<Model, ModelsError> {
        let this = self.clone();
        self.station.attach(async move {
            this.station
                .client
                .models()
                .get(&name)
                .await
                .map(model_from_wire)
                .map_err(|error| map_model_error(error, &name))
        })
    }

    fn list_versions(&self, model: String) -> Job<Vec<ModelVersion>, ModelsError> {
        let this = self.clone();
        self.station.attach(async move {
            this.station
                .client
                .models()
                .versions(&model)
                .await
                .map(model_versions_from_wire)
                .map_err(|error| map_model_error(error, &model))
        })
    }

    fn get_version(&self, model: String, spec: VersionSpec) -> Job<ModelVersion, ModelsError> {
        let this = self.clone();
        self.station.attach(async move {
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
    ) -> Job<Vec<Box<dyn VersionFileSource>>, ModelsError> {
        let route = match self.route_version(&model, &id) {
            Ok(route) => route,
            Err(error) => return Job::failed(error),
        };
        let this = self.clone();
        self.station.attach(async move {
            let station = &this.station;
            station
                .client
                .models()
                .download(&model, route)
                .await
                .map_err(|error| map_version_error(error, &model, &id))
                .map(|response| file_sources_from_wire(station, response))
        })
    }

    fn create_model(&self, name: String, description: Option<String>) -> Job<Model, ModelsError> {
        let this = self.clone();
        self.station.attach(async move {
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
    ) -> Job<ModelVersion, ModelsError> {
        Job::failed(ModelsError::other(
            "publishing a model version is not implemented for the station yet",
        ))
    }
}

struct StationVersionFileSource {
    file: VersionFile,
    url: String,
    station: Arc<StationInner>,
}

impl VersionFileSource for StationVersionFileSource {
    fn file(&self) -> &VersionFile {
        &self.file
    }

    fn open(&self, _canonical_path: String) -> Streaming<Bytes, TransferError> {
        let transfer = self.station.transfer.clone();
        let url = self.url.clone();
        let size = self.file.size_bytes;
        self.station.attach_stream(
            stream::once(async move { transfer.get(&url, Some(size)).await }).try_flatten(),
        )
    }
}

fn file_sources_from_wire(
    station: &Arc<StationInner>,
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
                station: Arc::clone(station),
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
