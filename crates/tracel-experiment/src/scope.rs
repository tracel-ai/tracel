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

/// A telemetry surface shared by experiment runs and activities.
///
/// Operations emitted through an activity guard or reference are attributed to that activity.
/// Operations emitted through a run or root handle remain at the run root.
pub trait ExperimentContext {
    /// Return an owned weak handle that preserves this context's scope.
    ///
    /// Custom context implementations should delegate this to an existing SDK context.
    fn scope_handle(&self) -> ExperimentRunHandle;

    /// Log a `trace`-level message.
    fn log_trace(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.log(LogRecord::trace(message))
    }

    /// Log a `debug`-level message.
    fn log_debug(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.log(LogRecord::debug(message))
    }

    /// Log an `info`-level message.
    fn log_info(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.log(LogRecord::info(message))
    }

    /// Log a `warn`-level message.
    fn log_warn(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.log(LogRecord::warn(message))
    }

    /// Log an `error`-level message.
    fn log_error(&self, message: impl Into<String>) -> Result<(), ExperimentError> {
        self.log(LogRecord::error(message))
    }

    /// Record a structured log entry.
    fn log(&self, record: LogRecord) -> Result<(), ExperimentError> {
        self.scope_handle().emit_log(record)
    }

    /// Log a named configuration object.
    fn log_config<C: Serialize>(
        &self,
        name: impl Into<String>,
        config: &C,
    ) -> Result<(), ExperimentError> {
        self.scope_handle().emit_config(name, config)
    }

    /// Log metric values for an epoch, split, and iteration.
    fn log_metric(
        &self,
        epoch: usize,
        split: impl Into<String>,
        iteration: usize,
        items: Vec<MetricValue>,
    ) -> Result<(), ExperimentError> {
        self.scope_handle()
            .emit_metric(epoch, split, iteration, items)
    }

    /// Log a metric definition.
    fn log_metric_definition(&self, spec: MetricSpec) -> Result<(), ExperimentError> {
        self.scope_handle().emit_metric_definition(spec)
    }

    /// Log aggregated metric values for an epoch and split.
    fn log_epoch_summary(
        &self,
        epoch: usize,
        split: impl Into<String>,
        items: Vec<MetricValue>,
    ) -> Result<(), ExperimentError> {
        self.scope_handle().emit_epoch_summary(epoch, split, items)
    }

    /// Log scalar summary values without an epoch axis.
    fn log_summary(&self, items: Vec<MetricValue>) -> Result<(), ExperimentError> {
        self.scope_handle().emit_summary(items)
    }

    /// Encode and persist an artifact.
    fn save_artifact<E: BundleEncode>(
        &self,
        name: impl AsRef<str>,
        kind: ArtifactKind,
        artifact: E,
        settings: &E::Settings,
    ) -> Result<(), ExperimentError> {
        self.scope_handle()
            .emit_save_artifact(name, kind, artifact, settings)
    }

    /// Load and decode an artifact from a compatible experiment identifier.
    fn use_artifact<D: BundleDecode>(
        &self,
        experiment_id: impl Into<ExperimentId>,
        name: impl AsRef<str>,
        settings: &D::Settings,
    ) -> Result<D, ExperimentError> {
        self.scope_handle()
            .emit_use_artifact(experiment_id, name, settings)
    }

    /// Create a child activity builder.
    fn activity(&self, name: impl Into<String>) -> ActivityBuilder {
        self.scope_handle().emit_activity(name)
    }
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

    fn emit_log(&self, mut record: LogRecord) -> Result<(), ExperimentError> {
        if !self.scope.is_empty() {
            record.inherit_attrs(&self.scope);
        }
        self.record_event(Event::Log {
            record,
            activity: self.activity,
        })
    }

    fn emit_config<C: Serialize>(
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

    fn emit_metric(
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

    fn emit_metric_definition(&self, spec: MetricSpec) -> Result<(), ExperimentError> {
        self.record_event(Event::MetricDefinition(spec))
    }

    fn emit_epoch_summary(
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

    fn emit_summary(&self, items: Vec<MetricValue>) -> Result<(), ExperimentError> {
        self.record_event(Event::Summary {
            items,
            activity: self.activity,
        })
    }

    fn emit_save_artifact<E: BundleEncode>(
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

    fn emit_use_artifact<D: BundleDecode>(
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

    fn emit_activity(&self, name: impl Into<String>) -> ActivityBuilder {
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
