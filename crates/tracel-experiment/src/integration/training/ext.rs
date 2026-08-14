use burn::train::{LearnerModel, SupervisedTraining};

use crate::ExperimentContext;

use super::ExperimentTrainingExt;

/// Extension methods for wiring experiment telemetry into supervised training.
pub trait SupervisedTrainingExperimentExt: Sized {
    /// Attach scoped metric logging, progress tracking, and cancellation.
    fn with_experiment<C>(self, context: &C) -> Self
    where
        C: ExperimentContext + ?Sized;

    /// Attach scoped model, optimizer, and scheduler checkpointers.
    fn with_experiment_checkpoints<C>(self, context: &C) -> Self
    where
        C: ExperimentContext + ?Sized;
}

impl<M> SupervisedTrainingExperimentExt for SupervisedTraining<M>
where
    M: LearnerModel,
{
    fn with_experiment<C>(self, context: &C) -> Self
    where
        C: ExperimentContext + ?Sized,
    {
        self.with_metric_logger(context.metric_logger())
            .with_progress_logger(context.training_progress_logger())
            .with_interrupter(context.interrupter())
    }

    fn with_experiment_checkpoints<C>(self, context: &C) -> Self
    where
        C: ExperimentContext + ?Sized,
    {
        let (model, optimizer, scheduler) = context.checkpointers();
        self.with_custom_checkpointers(model, optimizer, scheduler)
    }
}
