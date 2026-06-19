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
//! let _checkpoints = experiment.checkpoint_recorder();
//! let _interrupter = experiment.interrupter();
//! ```

mod checkpoint;
mod interrupter;
mod metric;
mod progress;

use std::path::PathBuf;

pub use checkpoint::ExperimentCheckpointer;
pub use interrupter::experiment_interrupter;
pub use metric::ExperimentMetricLogger;
pub use progress::{ExperimentEvaluationProgressLogger, ExperimentTrainingProgressLogger};

use crate::{ExperimentRun, integration::training::checkpoint::ExperimentCheckpointError};

/// Extension trait adding Burn `train` adapter constructors to [`ExperimentRun`].
pub trait ExperimentTrainingExt {
    /// Create a new [`ExperimentMetricLogger`] for this run.
    fn metric_logger(&self) -> ExperimentMetricLogger;

    /// Create a new [`ExperimentCheckpointRecorder`] for this run.
    fn checkpoint_recorder(
        &self,
        path: PathBuf,
    ) -> Result<ExperimentCheckpointer, ExperimentCheckpointError>;

    /// Create a new [`burn::train::Interrupter`] linked to this run's cancellation token.
    fn interrupter(&self) -> burn::train::Interrupter;

    /// Create a new [`ExperimentTrainingProgressLogger`] for this run.
    fn training_progress_logger(&self) -> ExperimentTrainingProgressLogger;

    /// Create a new [`ExperimentEvaluationProgressLogger`] for this run.
    fn evaluation_progress_logger(&self) -> ExperimentEvaluationProgressLogger;
}

impl ExperimentTrainingExt for ExperimentRun {
    fn metric_logger(&self) -> ExperimentMetricLogger {
        ExperimentMetricLogger::new(self)
    }

    fn checkpoint_recorder(
        &self,
        path: PathBuf,
    ) -> Result<ExperimentCheckpointer, ExperimentCheckpointError> {
        ExperimentCheckpointer::try_new(self, path)
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

impl ExperimentTrainingExt for crate::ExperimentRunHandle {
    fn metric_logger(&self) -> ExperimentMetricLogger {
        ExperimentMetricLogger::new(self.clone())
    }

    fn checkpoint_recorder(
        &self,
        path: PathBuf,
    ) -> Result<ExperimentCheckpointer, ExperimentCheckpointError> {
        ExperimentCheckpointer::try_new(self.clone(), path)
    }

    fn interrupter(&self) -> burn::train::Interrupter {
        experiment_interrupter(self.clone())
    }

    fn training_progress_logger(&self) -> ExperimentTrainingProgressLogger {
        ExperimentTrainingProgressLogger::new(self.clone())
    }

    fn evaluation_progress_logger(&self) -> ExperimentEvaluationProgressLogger {
        ExperimentEvaluationProgressLogger::new(self.clone())
    }
}
