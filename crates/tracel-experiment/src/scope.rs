//! Uniform telemetry contexts for experiment runs and activities.

use serde::Serialize;
use tracel_artifact::bundle::{BundleDecode, BundleEncode, FsBundle};

use crate::activity::{Activity, ActivityBuilder, ActivityGuard, ActivityId};
use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::session::Event;
use crate::{
    ArtifactKind, CancelToken, ExperimentId, ExperimentRun, ExperimentRunHandle, LogRecord,
    MetricSpec, MetricValue,
};

/// A bound for generic code that accepts any experiment-scoped emitter.
///
/// Runs, handles, activities, and guards implement this trait. Generic APIs such as
/// [`crate::integration::training::SupervisedTrainingExperimentExt::with_experiment`] use it to
/// preserve the caller's scope. Everyday telemetry is available through inherent methods on the
/// concrete types.
pub trait ExperimentContext {
    /// Return an owned weak handle that preserves this context's scope.
    ///
    /// Custom context implementations should delegate this to an existing SDK context.
    fn scope_handle(&self) -> ExperimentRunHandle;
}

impl ExperimentContext for ExperimentRun {
    fn scope_handle(&self) -> ExperimentRunHandle {
        self.handle.clone()
    }
}

impl ExperimentContext for ExperimentRunHandle {
    fn scope_handle(&self) -> ExperimentRunHandle {
        self.clone()
    }
}

impl ExperimentContext for ActivityGuard {
    fn scope_handle(&self) -> ExperimentRunHandle {
        self.activity.scope_handle()
    }
}

impl ExperimentContext for Activity {
    fn scope_handle(&self) -> ExperimentRunHandle {
        self.handle.clone()
    }
}

impl ExperimentRunHandle {
    pub(crate) fn for_activity(&self, activity: ActivityId, cancel_token: CancelToken) -> Self {
        Self {
            activity: Some(activity),
            context_cancel_token: cancel_token,
            ..self.clone()
        }
    }

    pub(crate) fn emit_log(&self, mut record: LogRecord) -> Result<(), ExperimentError> {
        if !self.scope.is_empty() {
            record.inherit_attrs(&self.scope);
        }
        self.record_event(Event::Log {
            record,
            activity: self.activity,
        })
    }

    pub(crate) fn emit_config<C: Serialize>(
        &self,
        name: impl Into<String>,
        config: &C,
    ) -> Result<(), ExperimentError> {
        let value = serde_json::to_value(config).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to serialize experiment config",
                error,
            )
        })?;

        self.record_event(Event::Config {
            name: name.into(),
            value,
        })
    }

    pub(crate) fn emit_metric(
        &self,
        epoch: usize,
        split: impl Into<String>,
        iteration: usize,
        items: Vec<MetricValue>,
    ) -> Result<(), ExperimentError> {
        self.record_event(Event::Metrics {
            epoch,
            split: split.into(),
            iteration,
            items,
            activity: self.activity,
        })
    }

    pub(crate) fn emit_metric_definition(&self, spec: MetricSpec) -> Result<(), ExperimentError> {
        self.record_event(Event::MetricDefinition(spec))
    }

    pub(crate) fn emit_epoch_summary(
        &self,
        epoch: usize,
        split: impl Into<String>,
        items: Vec<MetricValue>,
    ) -> Result<(), ExperimentError> {
        self.record_event(Event::EpochSummary {
            epoch,
            split: split.into(),
            items,
            activity: self.activity,
        })
    }

    pub(crate) fn emit_summary(&self, items: Vec<MetricValue>) -> Result<(), ExperimentError> {
        self.record_event(Event::Summary {
            items,
            activity: self.activity,
        })
    }

    pub(crate) fn emit_save_artifact<E: BundleEncode>(
        &self,
        name: impl AsRef<str>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<(), ExperimentError> {
        let inner = self.upgrade()?;
        inner.ensure_active()?;

        let artifact_fn = |bundle: &mut FsBundle| {
            artifact.encode(bundle, settings).map_err(|error| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    "Failed to encode artifact into bundle",
                    error,
                )
            })
        };

        inner
            .session
            .save_artifact(name.as_ref(), kind, self.activity, Box::new(artifact_fn))
    }

    pub(crate) fn emit_use_artifact<D: BundleDecode>(
        &self,
        experiment_id: impl Into<ExperimentId>,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentError> {
        let inner = self.upgrade()?;
        inner.ensure_active()?;
        let name = name.as_ref();
        let artifact = inner
            .reader
            .load_artifact_raw(experiment_id.into(), name)
            .map_err(|error| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    format!("Failed to load artifact bundle for {name}"),
                    error,
                )
            })?;

        D::decode(&artifact.bundle, settings).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                format!("Failed to decode artifact: {name}"),
                error,
            )
        })
    }

    pub(crate) fn emit_activity(&self, name: impl Into<String>) -> ActivityBuilder {
        let inner = match self.upgrade() {
            Ok(inner) => inner,
            Err(_) => {
                return ActivityBuilder::detached(
                    name,
                    self.activity,
                    self.context_cancel_token.clone(),
                    self.clone(),
                );
            }
        };

        ActivityBuilder::new(
            inner.activity_id_allocator.clone(),
            self.control.clone(),
            name.into(),
            self.clone(),
        )
        .with_parent(self.activity, self.context_cancel_token.clone())
    }
}
