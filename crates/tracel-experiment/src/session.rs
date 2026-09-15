use std::sync::Arc;

use tracel_artifact::bundle::FsBundle;
use tracel_task::Job;

use crate::{
    ArtifactKind, ExperimentId, MetricSpec, MetricValue,
    activity::{ActivityEvent, ActivityId},
    error::ExperimentError,
    log::LogRecord,
    reader::ArtifactRef,
};

#[derive(Debug, Clone)]
pub enum Event {
    Args(serde_json::Value),
    Config {
        name: String,
        value: serde_json::Value,
    },
    Log {
        record: LogRecord,
        activity: Option<ActivityId>,
    },
    Metrics {
        epoch: usize,
        split: String,
        iteration: usize,
        items: Vec<MetricValue>,
        activity: Option<ActivityId>,
    },
    MetricDefinition(MetricSpec),
    EpochSummary {
        epoch: usize,
        split: String,
        items: Vec<MetricValue>,
        activity: Option<ActivityId>,
    },
    Summary {
        items: Vec<MetricValue>,
        activity: Option<ActivityId>,
    },
    ArtifactUsed {
        experiment_id: ExperimentId,
        reference: ArtifactRef,
    },
    Activity(ActivityEvent),
}

/// Final completion state recorded for an experiment run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentCompletion {
    /// The run completed successfully.
    Success,

    /// The run failed with the provided reason.
    Failed(String),

    /// The run was cancelled before completion.
    Cancelled,
}

/// Session-level implementation for the active experiment run.
///
/// A session is a synchronous producer with asynchronous drains: [`record_event`](Self::record_event)
/// never waits, and [`flush`](Self::flush) and [`finish`](Self::finish) hand their request to the
/// backend before returning, so a run dropped mid-way still completes without anyone driving the
/// job they return.
pub trait ExperimentSession: Send + Sync {
    /// Queues `event`; never waits.
    fn record_event(&self, event: Event) -> Result<(), ExperimentError>;

    /// Resolves once every event recorded so far has left the process.
    fn flush(&self) -> Job<(), ExperimentError> {
        Job::ready(())
    }

    /// Ships an artifact already encoded into `bundle`.
    fn save_artifact(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Job<(), ExperimentError>;

    /// Hands the completion to the backend before returning; the job resolves once the backend
    /// has acknowledged it.
    fn finish(&self, completion: ExperimentCompletion) -> Job<(), ExperimentError>;
}

impl<T> ExperimentSession for Arc<T>
where
    T: ExperimentSession,
{
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.as_ref().record_event(event)
    }

    fn flush(&self) -> Job<(), ExperimentError> {
        self.as_ref().flush()
    }

    fn save_artifact(
        &self,
        name: String,
        kind: ArtifactKind,
        bundle: FsBundle,
    ) -> Job<(), ExperimentError> {
        self.as_ref().save_artifact(name, kind, bundle)
    }

    fn finish(&self, completion: ExperimentCompletion) -> Job<(), ExperimentError> {
        self.as_ref().finish(completion)
    }
}
