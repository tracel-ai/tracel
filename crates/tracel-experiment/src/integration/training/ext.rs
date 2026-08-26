use burn::train::{LearnerModel, SupervisedTraining};

use crate::ExperimentRunHandle;

use super::{
    ExperimentCheckpointer, ExperimentMetricLogger, ExperimentTrainingProgressLogger,
    experiment_interrupter,
};

/// Extension methods for wiring experiment telemetry into supervised training.
pub trait SupervisedTrainingExperimentExt: Sized {
    /// Attach scoped metric logging, progress tracking, and cancellation.
    fn with_experiment(self, experiment: impl Into<ExperimentRunHandle>) -> Self;

    /// Attach scoped model, optimizer, and scheduler checkpointers.
    fn with_experiment_checkpoints(self, experiment: impl Into<ExperimentRunHandle>) -> Self;
}

impl<M> SupervisedTrainingExperimentExt for SupervisedTraining<M>
where
    M: LearnerModel,
{
    fn with_experiment(self, experiment: impl Into<ExperimentRunHandle>) -> Self {
        let handle = experiment.into();
        self.with_metric_logger(ExperimentMetricLogger::new(handle.clone()))
            .with_progress_logger(ExperimentTrainingProgressLogger::new(handle.clone()))
            .with_interrupter(experiment_interrupter(handle))
    }

    fn with_experiment_checkpoints(self, experiment: impl Into<ExperimentRunHandle>) -> Self {
        let handle = experiment.into();
        let model = ExperimentCheckpointer::new(handle.clone(), "model".to_string());
        let optimizer = ExperimentCheckpointer::new(handle.clone(), "optim".to_string());
        let scheduler = ExperimentCheckpointer::new(handle, "scheduler".to_string());
        self.with_custom_checkpointers(model, optimizer, scheduler)
    }
}
