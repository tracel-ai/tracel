use burn::train::logger::{EvaluationProgressLogger, TrainingProgressLogger};

use crate::{ExperimentRunHandle, progress::ProgressGuard};

/// Experiment-backed implementation of Burn's [`TrainingProgressLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::training_progress_logger`] when
/// you already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentTrainingProgressLogger {
    experiment: ExperimentRunHandle,
    training_guard: Option<ProgressGuard>,
    split_guard: Option<ProgressGuard>,
}

impl ExperimentTrainingProgressLogger {
    /// Create a training progress logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment: experiment.into(),
            training_guard: None,
            split_guard: None,
        }
    }
}

impl TrainingProgressLogger for ExperimentTrainingProgressLogger {
    fn start(&mut self, total_epochs: usize, _total_items: Option<usize>) {
        self.training_guard = Some(
            self.experiment
                .progress("Training")
                .total(total_epochs as u64)
                .unit("epochs")
                .start(),
        );
    }

    fn start_split(&mut self, name: &str, total_items: usize) {
        let builder = if let Some(guard) = &self.training_guard {
            guard.child(name)
        } else {
            self.experiment.progress(name)
        };
        self.split_guard = Some(builder.total(total_items as u64).unit("steps").start());
    }

    fn update_split(&mut self, items_processed: usize) {
        if let Some(guard) = &mut self.split_guard {
            guard.set(items_processed as u64);
        }
    }

    fn end_split(&mut self) {
        if let Some(guard) = self.split_guard.take() {
            guard.finish();
        }
    }

    fn update_epoch(&mut self, epoch: usize) {
        if let Some(guard) = &mut self.training_guard {
            guard.set(epoch as u64);
        }
    }

    fn end(&mut self) {
        if let Some(guard) = self.training_guard.take() {
            guard.finish();
        }
    }

    fn log_event_training(&mut self, _event: String) {} // no-op
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
    fn start_global_progress(&mut self, total_tests: usize) {
        self.eval_guard = Some(
            self.experiment
                .progress("Evaluation")
                .total(total_tests as u64)
                .unit("tests")
                .start(),
        );
    }

    fn start_test(&mut self, name: &str, total_items: usize) {
        let builder = if let Some(guard) = &self.eval_guard {
            guard.child(name)
        } else {
            self.experiment.progress(name)
        };
        self.test_guard = Some(builder.total(total_items as u64).unit("steps").start());
    }

    fn update_test_progress(&mut self, items_processed: usize) {
        if let Some(guard) = &mut self.test_guard {
            guard.set(items_processed as u64);
        }
    }

    fn end_test(&mut self) {
        if let Some(guard) = self.test_guard.take() {
            guard.finish();
        }
    }

    fn end_global_progress(&mut self) {
        if let Some(guard) = self.eval_guard.take() {
            guard.finish();
        }
    }

    fn log_event_evaluation(&mut self, _event: String) {} // no-op
}
