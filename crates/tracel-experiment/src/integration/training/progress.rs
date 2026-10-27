use burn::data::dataloader::Progress;
use burn::train::logger::{EvaluationProgressLogger, TrainingProgressLogger};

use crate::{ActivityBuilder, ActivityGuard, ExperimentRunHandle};

/// What a split or a test is metered in where burn names no unit.
const DEFAULT_UNIT: &str = "steps";

/// Starts the activity a split or a test runs as, metered against its total where burn knows
/// it and open-ended where it does not.
fn start_metered(builder: ActivityBuilder, progress: &Progress) -> ActivityGuard {
    let builder = match progress.total {
        Some(total) => builder.meter(
            total as u64,
            progress.unit.as_deref().unwrap_or(DEFAULT_UNIT),
        ),
        None => builder,
    };
    let guard = builder.start();
    if progress.items_processed > 0 {
        guard.set(progress.items_processed as u64);
    }
    guard
}

/// Whether a split's or a test's final snapshot says it ran to the end. Burn ends one whether
/// its loader ran dry or died under it, and only the count says which; one of unknown length is
/// taken as finished, there being nothing to compare it with.
fn ran_to_the_end(progress: &Progress) -> bool {
    progress
        .total
        .is_none_or(|total| progress.items_processed >= total)
}

/// Experiment-backed implementation of Burn's [`TrainingProgressLogger`] trait.
///
/// Prefer [`crate::integration::training::ExperimentTrainingExt::training_progress_logger`] when
/// you already have an [`ExperimentRun`][crate::ExperimentRun] in scope.
pub struct ExperimentTrainingProgressLogger {
    experiment: ExperimentRunHandle,
    name: String,
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
            name: "Training".to_string(),
            training_guard: None,
            epoch_guard: None,
            split_guard: None,
            completed_epochs: 0,
            total_epochs: None,
        }
    }

    /// Set the name of the top-level activity, `"Training"` by default.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
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
    fn start(
        &mut self,
        total_epochs: usize,
        starting_epoch: usize,
        _total_items: Option<usize>,
        label: Option<&str>,
    ) {
        self.completed_epochs = starting_epoch.saturating_sub(1);
        self.total_epochs = Some(total_epochs);
        self.epoch_guard = None;
        self.split_guard = None;
        self.training_guard = Some(
            self.experiment
                .activity(label.unwrap_or(self.name.as_str()))
                .meter(total_epochs as u64, "epochs")
                .start(),
        );
    }

    fn start_split(&mut self, name: &str, progress: &Progress) {
        self.ensure_epoch_scope();

        let builder = if let Some(epoch_guard) = &self.epoch_guard {
            epoch_guard.activity(name)
        } else if let Some(guard) = &self.training_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.split_guard = Some(start_metered(builder, progress));
    }

    fn update_split(&mut self, progress: &Progress) {
        if let Some(guard) = &self.split_guard {
            guard.set(progress.items_processed as u64);
        }
    }

    fn end_split(&mut self, progress: &Progress) {
        if let Some(guard) = self.split_guard.take() {
            match ran_to_the_end(progress) {
                true => guard.finish(),
                false => guard.abandon(),
            }
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
            let done = self
                .total_epochs
                .is_some_and(|total| self.completed_epochs >= total);
            match done {
                true => guard.finish(),
                false => guard.abandon(),
            }
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
    name: String,
    eval_guard: Option<ActivityGuard>,
    test_guard: Option<ActivityGuard>,
    tests_total: usize,
    tests_done: usize,
}

impl ExperimentEvaluationProgressLogger {
    /// Create an evaluation progress logger backed by the provided experiment run.
    pub fn new(experiment: impl Into<ExperimentRunHandle>) -> Self {
        Self {
            experiment: experiment.into(),
            name: "Evaluation".to_string(),
            eval_guard: None,
            test_guard: None,
            tests_total: 0,
            tests_done: 0,
        }
    }

    /// Set the name of the top-level activity, `"Evaluation"` by default.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }
}

impl EvaluationProgressLogger for ExperimentEvaluationProgressLogger {
    fn start_global_progress(&mut self, total_tests: usize) {
        self.tests_total = total_tests;
        self.tests_done = 0;
        self.eval_guard = Some(
            self.experiment
                .activity(self.name.clone())
                .meter(total_tests as u64, "tests")
                .start(),
        );
    }

    fn start_test(&mut self, name: &str, progress: &Progress) {
        let builder = if let Some(guard) = &self.eval_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.test_guard = Some(start_metered(builder, progress));
    }

    fn update_test_progress(&mut self, progress: &Progress) {
        if let Some(guard) = &self.test_guard {
            guard.set(progress.items_processed as u64);
        }
    }

    fn end_test(&mut self, progress: &Progress) {
        if let Some(guard) = self.test_guard.take() {
            match ran_to_the_end(progress) {
                true => {
                    self.tests_done += 1;
                    if let Some(eval) = &self.eval_guard {
                        eval.set(self.tests_done as u64);
                    }
                    guard.finish();
                }
                false => guard.abandon(),
            }
        }
    }

    fn end_global_progress(&mut self) {
        self.test_guard.take();
        if let Some(guard) = self.eval_guard.take() {
            match self.tests_done >= self.tests_total {
                true => guard.finish(),
                false => guard.abandon(),
            }
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

    /// A snapshot of `items_processed` out of a known `total`, in burn's default unit.
    fn progress(items_processed: usize, total: usize) -> Progress {
        Progress::new(items_processed, total, None)
    }

    #[test]
    fn training_progress_groups_splits_under_supplied_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("Fold 1").start();
        let parent_id = parent.id();
        let mut logger = parent.training_progress_logger();

        logger.start(2, 1, None, None);
        logger.start_split("train", &progress(0, 10));
        logger.update_split(&progress(4, 10));
        logger.end_split(&progress(4, 10));
        logger.start_split("valid", &progress(0, 5));
        logger.end_split(&progress(0, 5));
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

    /// A loader that cannot tell its length meters nothing, and its split is taken as finished:
    /// there is no total to fall short of.
    #[test]
    fn a_split_of_unknown_length_is_open_ended_and_finishes() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let mut logger = run.training_progress_logger();

        logger.start(1, 1, None, None);
        logger.start_split("train", &Progress::new(0, None, None));
        logger.end_split(&Progress::new(7, None, None));

        let events = session.activity_events();
        let split_started = events.iter().find_map(|event| match event {
            ActivityEvent::Started { activity } if activity.name == "train" => Some(activity),
            _ => None,
        });
        assert!(split_started.expect("the split started").meter.is_none());
        assert!(events.iter().any(|event| matches!(
            event,
            ActivityEvent::Finished {
                status: crate::ActivityStatus::Success,
                ..
            }
        )));
    }

    #[test]
    fn training_progress_uses_supplied_name() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let mut logger = run.training_progress_logger().with_name("Pretraining");

        logger.start(1, 1, None, None);
        logger.end();

        let names = session
            .activity_events()
            .into_iter()
            .filter_map(|event| match event {
                ActivityEvent::Started { activity } => Some(activity.name),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["Pretraining"]);
    }

    #[test]
    fn endings_follow_the_meter() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("Fold 6").start();
        let mut logger = parent.training_progress_logger();

        logger.start(2, 1, None, None);
        logger.start_split("train", &progress(0, 10));
        logger.update_split(&progress(10, 10));
        logger.end_split(&progress(10, 10));
        logger.start_split("valid", &progress(0, 10));
        logger.update_split(&progress(3, 10));
        logger.end_split(&progress(3, 10));
        logger.end();

        let completions = session
            .activity_events()
            .into_iter()
            .filter_map(|event| match event {
                ActivityEvent::Finished { id, status, .. } => Some((id, status)),
                _ => None,
            })
            .collect::<Vec<_>>();

        let statuses: Vec<_> = completions.iter().map(|(_, s)| s.clone()).collect();
        assert!(
            matches!(
                statuses.as_slice(),
                [
                    crate::ActivityStatus::Success,   // the train split ran dry
                    crate::ActivityStatus::Abandoned, // the valid split stopped at 3 of 10
                    crate::ActivityStatus::Abandoned, // the epoch never completed
                    crate::ActivityStatus::Abandoned, // 0 of 2 epochs is not a finished training
                ]
            ),
            "unexpected endings: {statuses:?}"
        );
    }

    #[test]
    fn evaluation_progress_groups_tests_under_supplied_scope() {
        let session = Arc::new(MockSession::default());
        let run = create_run(session.clone());
        let parent = run.activity("Fold 1").start();
        let mut logger = parent.evaluation_progress_logger().with_name("Holdout");

        logger.start_global_progress(2);
        logger.start_test("test", &progress(0, 4));
        logger.update_test_progress(&progress(2, 4));
        logger.end_test(&progress(2, 4));
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
        assert_eq!(evaluation.name, "Holdout");
        assert_eq!(evaluation.parent, Some(parent.id()));
        assert_eq!(evaluation.meter.as_ref().unwrap().total, Some(2));
        assert_eq!(test.parent, Some(evaluation.id));
        assert_eq!(test.meter.as_ref().unwrap().total, Some(4));
    }
}
