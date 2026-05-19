use burn::train::logger::{EvaluationProgressLogger, TrainingProgressLogger};
use burn::train::renderer::{EvaluationProgress, TrainingProgress};

use crate::{ExperimentRunHandle, progress::ProgressGuard};

/// Experiment-backed implementation of Burn's [`TrainingProgressLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::training_progress_logger`] when
/// you already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentTrainingProgressLogger {
    experiment: ExperimentRunHandle,
    training_guard: Option<ProgressGuard>,
    epoch_guard: Option<ProgressGuard>,
    train_guard: Option<ProgressGuard>,
    valid_guard: Option<ProgressGuard>,
}

impl ExperimentTrainingProgressLogger {
    /// Create a training progress logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment: experiment.into(),
            training_guard: None,
            epoch_guard: None,
            train_guard: None,
            valid_guard: None,
        }
    }
}

impl TrainingProgressLogger for ExperimentTrainingProgressLogger {
    fn update_train(&mut self, progress: &TrainingProgress) {
        if self.training_guard.is_none() {
            self.training_guard = Some(
                self.experiment
                    .progress("Training")
                    .total(progress.global_progress.items_total as u64)
                    .unit("steps")
                    .start(),
            );
        }

        if self.epoch_guard.is_none() {
            let builder = if let Some(training) = &self.training_guard {
                training.child("Epoch")
            } else {
                self.experiment.progress("Epoch")
            };
            let builder = match &progress.progress {
                Some(p) => builder.total(p.items_total as u64).unit("steps"),
                None => builder.unit("steps"),
            };
            self.epoch_guard = Some(builder.start());
        }

        if self.train_guard.is_none() {
            let builder = if let Some(epoch) = &self.epoch_guard {
                epoch.child("Train")
            } else {
                self.experiment.progress("Train")
            };
            let builder = match &progress.progress {
                Some(p) => builder.total(p.items_total as u64).unit("steps"),
                None => builder.unit("steps"),
            };
            self.train_guard = Some(builder.start());
        }

        if let Some(guard) = &mut self.train_guard {
            if let Some(p) = &progress.progress {
                guard.set(p.items_processed as u64);
            }
        }
        if let Some(guard) = &mut self.training_guard {
            guard.set(progress.global_progress.items_processed as u64);
        }
    }

    fn update_valid(&mut self, progress: &TrainingProgress) {
        if self.valid_guard.is_none() {
            if let Some(guard) = self.train_guard.take() {
                guard.finish();
            }

            let builder = if let Some(epoch) = &self.epoch_guard {
                epoch.child("Valid")
            } else {
                self.experiment.progress("Valid")
            };
            let builder = match &progress.progress {
                Some(p) => builder.total(p.items_total as u64).unit("steps"),
                None => builder.unit("steps"),
            };
            self.valid_guard = Some(builder.start());
        }

        if let Some(guard) = &mut self.valid_guard {
            if let Some(p) = &progress.progress {
                guard.set(p.items_processed as u64);
            }
        }
    }

    fn end_epoch(&mut self, epoch: usize) {
        if let Some(guard) = self.valid_guard.take() {
            guard.finish();
        }
        if let Some(guard) = self.train_guard.take() {
            guard.finish();
        }
        if let Some(guard) = self.epoch_guard.take() {
            guard.finish_with_message(format!("Epoch {epoch} complete"));
        }
    }
}

/// Experiment-backed implementation of Burn's [`EvaluationProgressLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::evaluation_progress_logger`] when
/// you already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentEvaluationProgressLogger {
    experiment: ExperimentRunHandle,
    eval_guard: Option<ProgressGuard>,
    test_guard: Option<ProgressGuard>,
}

impl ExperimentEvaluationProgressLogger {
    /// Create an evaluation progress logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment: experiment.into(),
            eval_guard: None,
            test_guard: None,
        }
    }
}

impl EvaluationProgressLogger for ExperimentEvaluationProgressLogger {
    fn update_test(&mut self, progress: &EvaluationProgress) {
        if self.eval_guard.is_none() {
            self.eval_guard = Some(
                self.experiment
                    .progress("Evaluation")
                    .total(progress.progress.items_total as u64)
                    .unit("steps")
                    .start(),
            );
        }

        if self.test_guard.is_none() {
            let builder = if let Some(eval) = &self.eval_guard {
                eval.child("Test")
            } else {
                self.experiment.progress("Test")
            };
            self.test_guard = Some(
                builder
                    .total(progress.progress.items_total as u64)
                    .unit("steps")
                    .start(),
            );
        }

        if let Some(guard) = &mut self.test_guard {
            guard.set(progress.progress.items_processed as u64);
        }
    }

    fn end_eval(&mut self) {
        if let Some(guard) = self.test_guard.take() {
            guard.finish();
        }
        if let Some(guard) = self.eval_guard.take() {
            guard.finish();
        }
    }
}
