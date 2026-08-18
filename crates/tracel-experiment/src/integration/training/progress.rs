use burn::train::logger::{EvaluationProgressLogger, TrainingProgressLogger};

use crate::{ActivityGuard, ExperimentRunHandle};

/// Experiment-backed implementation of Burn's [`TrainingProgressLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::training_progress_logger`] when
/// you already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentTrainingProgressLogger {
    experiment: ExperimentRunHandle,
    training_guard: Option<ActivityGuard>,
    epoch_guard: Option<ActivityGuard>,
    split_guard: Option<ActivityGuard>,
    completed_epochs: usize,
    total_epochs: Option<usize>,
}

impl ExperimentTrainingProgressLogger {
    /// Create a training progress logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment: experiment.into(),
            training_guard: None,
            epoch_guard: None,
            split_guard: None,
            completed_epochs: 0,
            total_epochs: None,
        }
    }

    fn ensure_epoch_scope(&mut self) {
        if self.epoch_guard.is_some() {
            return;
        }

        let epoch = self.completed_epochs + 1;
        let name = format!("Epoch {epoch}");
        let builder = if let Some(guard) = &self.training_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };

        let mut builder = builder.attr("activity_type", "epoch").attr("epoch", epoch);

        if let Some(total_epochs) = self.total_epochs {
            builder = builder.attr("total_epochs", total_epochs);
        }

        self.epoch_guard = Some(builder.start());
    }
}

impl TrainingProgressLogger for ExperimentTrainingProgressLogger {
    fn start(&mut self, total_epochs: usize, starting_epoch: usize, _total_items: Option<usize>) {
        self.completed_epochs = starting_epoch.saturating_sub(1);
        self.total_epochs = Some(total_epochs);
        self.epoch_guard = None;
        self.split_guard = None;
        self.training_guard = Some(
            self.experiment
                .activity("Training")
                .meter(total_epochs as u64, "epochs")
                .start(),
        );
    }

    fn start_split(&mut self, name: &str, total_items: usize) {
        self.ensure_epoch_scope();

        let builder = if let Some(epoch_guard) = &self.epoch_guard {
            epoch_guard.activity(name)
        } else if let Some(guard) = &self.training_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.split_guard = Some(builder.meter(total_items as u64, "steps").start());
    }

    fn update_split(&mut self, items_processed: usize) {
        if let Some(guard) = &self.split_guard {
            guard.set(items_processed as u64);
        }
    }

    fn end_split(&mut self) {
        if let Some(guard) = self.split_guard.take() {
            guard.finish();
        }
    }

    fn update_epoch(&mut self, epoch: usize) {
        self.completed_epochs = epoch;

        if let Some(guard) = &self.training_guard {
            guard.set(epoch as u64);
        }

        if let Some(guard) = self.epoch_guard.take() {
            guard.finish();
        }
    }

    fn end(&mut self) {
        self.split_guard.take();
        self.epoch_guard.take();

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
    eval_guard: Option<ActivityGuard>,
    test_guard: Option<ActivityGuard>,
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
                .activity("Evaluation")
                .meter(total_tests as u64, "tests")
                .start(),
        );
    }

    fn start_test(&mut self, name: &str, total_items: usize) {
        let builder = if let Some(guard) = &self.eval_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.test_guard = Some(builder.meter(total_items as u64, "steps").start());
    }

    fn update_test_progress(&mut self, items_processed: usize) {
        if let Some(guard) = &self.test_guard {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        activity::ActivityEvent,
        integration::training::ExperimentTrainingExt,
        session::Event,
        test_support::{MockSession, create_run},
    };

    use super::*;

    #[test]
    fn training_progress_groups_splits_under_supplied_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("Fold 1").start();
        let parent_id = parent.id();
        let mut logger = parent.training_progress_logger();

        logger.start(2, 1, None);
        logger.start_split("train", 10);
        logger.update_split(4);
        logger.end_split();
        logger.start_split("valid", 5);
        logger.end_split();
        logger.update_epoch(1);
        logger.end();

        let events = session.events.lock().unwrap();
        let started = events
            .iter()
            .filter_map(|event| match event {
                Event::Activity(ActivityEvent::Started { activity }) => Some(activity),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(started.len(), 5);
        let training = started[1];
        let epoch = started[2];
        let train = started[3];
        let valid = started[4];

        assert_eq!(training.name, "Training");
        assert_eq!(training.parent, Some(parent_id));
        assert_eq!(
            training.meter.as_ref().unwrap().unit.as_deref(),
            Some("epochs")
        );
        assert_eq!(training.meter.as_ref().unwrap().total, Some(2));

        assert_eq!(epoch.name, "Epoch 1");
        assert_eq!(epoch.parent, Some(training.id));
        assert!(epoch.meter.is_none());
        assert_eq!(
            epoch.attributes.get("activity_type"),
            Some(&serde_json::json!("epoch"))
        );
        assert_eq!(epoch.attributes.get("epoch"), Some(&serde_json::json!(1)));

        assert_eq!(train.name, "train");
        assert_eq!(train.parent, Some(epoch.id));
        assert_eq!(train.meter.as_ref().unwrap().unit.as_deref(), Some("steps"));
        assert_eq!(train.meter.as_ref().unwrap().total, Some(10));

        assert_eq!(valid.name, "valid");
        assert_eq!(valid.parent, Some(epoch.id));
        assert_eq!(valid.meter.as_ref().unwrap().unit.as_deref(), Some("steps"));
        assert_eq!(valid.meter.as_ref().unwrap().total, Some(5));
    }

    #[test]
    fn evaluation_progress_groups_tests_under_supplied_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("Fold 1").start();
        let mut logger = parent.evaluation_progress_logger();

        logger.start_global_progress(2);
        logger.start_test("test", 4);
        logger.update_test_progress(2);
        logger.end_test();
        logger.end_global_progress();

        let events = session.events.lock().unwrap();
        let started = events
            .iter()
            .filter_map(|event| match event {
                Event::Activity(ActivityEvent::Started { activity }) => Some(activity),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(started.len(), 3);
        let evaluation = started[1];
        let test = started[2];
        assert_eq!(evaluation.parent, Some(parent.id()));
        assert_eq!(evaluation.meter.as_ref().unwrap().total, Some(2));
        assert_eq!(test.parent, Some(evaluation.id));
        assert_eq!(test.meter.as_ref().unwrap().total, Some(4));
    }
}
