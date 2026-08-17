//! Burn `train` adapters backed by an [`crate::ExperimentRun`].
//!
//! These adapters let learners emit metrics, write checkpoints, and respond to experiment
//! cancellation without each training loop needing to know about the underlying experiment
//! backend.
//!
//! Import [`ExperimentTrainingExt`] for the ergonomic constructors, or use the concrete adapter
//! types directly.
//!
//! # Example
//!
//! ```ignore
//! use tracel_experiment::ExperimentRun;
//! use tracel_experiment::integration::training::ExperimentTrainingExt;
//!
//! let experiment = ExperimentRun::local("./runs").unwrap();
//!
//! let _metrics = experiment.metric_logger();
//! let _checkpoints = experiment.checkpointers();
//! let _interrupter = experiment.interrupter();
//! ```

mod checkpoint;
mod ext;
mod interrupter;
mod metric;
mod progress;

pub use checkpoint::ExperimentCheckpointer;
pub use ext::SupervisedTrainingExperimentExt;
pub use interrupter::experiment_interrupter;
pub use metric::ExperimentMetricLogger;
pub use progress::{ExperimentEvaluationProgressLogger, ExperimentTrainingProgressLogger};

use crate::{ExperimentId, ExperimentRunHandle};

/// Extension trait adding Burn `train` adapters to experiment scopes.
pub trait ExperimentTrainingExt {
    /// Create a new [`ExperimentMetricLogger`] for this context.
    fn metric_logger(&self) -> ExperimentMetricLogger;

    /// Create the three checkpointers (model, optimizer, lr scheduler) for supervised training.
    fn checkpointers(
        &self,
    ) -> (
        ExperimentCheckpointer,
        ExperimentCheckpointer,
        ExperimentCheckpointer,
    );

    /// Create the three checkpointers configured to restore from a previous experiment.
    ///
    /// Saves still go to the current experiment, but `restore` loads from `source_id`.
    fn checkpointers_from(
        &self,
        source_id: impl Into<ExperimentId>,
    ) -> (
        ExperimentCheckpointer,
        ExperimentCheckpointer,
        ExperimentCheckpointer,
    );

    /// Create a new [`burn::train::Interrupter`] linked to this context's cancellation token.
    fn interrupter(&self) -> burn::train::Interrupter;

    /// Create a new [`ExperimentTrainingProgressLogger`] for this context.
    fn training_progress_logger(&self) -> ExperimentTrainingProgressLogger;

    /// Create a new [`ExperimentEvaluationProgressLogger`] for this context.
    fn evaluation_progress_logger(&self) -> ExperimentEvaluationProgressLogger;
}

impl<T> ExperimentTrainingExt for T
where
    for<'a> &'a T: Into<ExperimentRunHandle>,
{
    fn metric_logger(&self) -> ExperimentMetricLogger {
        ExperimentMetricLogger::new(self)
    }

    fn checkpointers(
        &self,
    ) -> (
        ExperimentCheckpointer,
        ExperimentCheckpointer,
        ExperimentCheckpointer,
    ) {
        let handle: ExperimentRunHandle = self.into();
        (
            ExperimentCheckpointer::new(handle.clone(), "model".to_string()),
            ExperimentCheckpointer::new(handle.clone(), "optim".to_string()),
            ExperimentCheckpointer::new(handle, "scheduler".to_string()),
        )
    }

    fn checkpointers_from(
        &self,
        source_id: impl Into<ExperimentId>,
    ) -> (
        ExperimentCheckpointer,
        ExperimentCheckpointer,
        ExperimentCheckpointer,
    ) {
        let handle: ExperimentRunHandle = self.into();
        let id = source_id.into();
        (
            ExperimentCheckpointer::new(handle.clone(), "model".to_string())
                .with_restore_from(id.clone()),
            ExperimentCheckpointer::new(handle.clone(), "optim".to_string())
                .with_restore_from(id.clone()),
            ExperimentCheckpointer::new(handle, "scheduler".to_string()).with_restore_from(id),
        )
    }

    fn interrupter(&self) -> burn::train::Interrupter {
        experiment_interrupter(self)
    }

    fn training_progress_logger(&self) -> ExperimentTrainingProgressLogger {
        ExperimentTrainingProgressLogger::new(self)
    }

    fn evaluation_progress_logger(&self) -> ExperimentEvaluationProgressLogger {
        ExperimentEvaluationProgressLogger::new(self)
    }
}
