use burn::train::logger::{EvaluationProgressLogger, TrainingProgressLogger};

use crate::{ActivityGuard, ExperimentRunHandle};

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
    split_seen: usize,
    split_total: usize,
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
            split_seen: 0,
            split_total: 0,
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

    fn start_split(&mut self, name: &str, total_items: usize) {
        self.ensure_epoch_scope();

        let builder = if let Some(epoch_guard) = &self.epoch_guard {
            epoch_guard.activity(name)
        } else if let Some(guard) = &self.training_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.split_seen = 0;
        self.split_total = total_items;
        self.split_guard = Some(builder.meter(total_items as u64, "steps").start());
    }

    fn update_split(&mut self, items_processed: usize) {
        self.split_seen = items_processed;
        if let Some(guard) = &self.split_guard {
            guard.set(items_processed as u64);
        }
    }

    fn end_split(&mut self) {
        if let Some(guard) = self.split_guard.take() {
            // Burn ends a split whether its loader ran dry or died under it;
            // only the meter says which.
            match self.split_seen >= self.split_total {
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
    test_seen: usize,
    test_total: usize,
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
            test_seen: 0,
            test_total: 0,
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

    fn start_test(&mut self, name: &str, total_items: usize) {
        let builder = if let Some(guard) = &self.eval_guard {
            guard.activity(name)
        } else {
            self.experiment.activity(name)
        };
        self.test_seen = 0;
        self.test_total = total_items;
        self.test_guard = Some(builder.meter(total_items as u64, "steps").start());
    }

    fn update_test_progress(&mut self, items_processed: usize) {
        self.test_seen = items_processed;
        if let Some(guard) = &self.test_guard {
            guard.set(items_processed as u64);
        }
    }

    fn end_test(&mut self) {
        if let Some(guard) = self.test_guard.take() {
            match self.test_seen >= self.test_total {
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
    use rstest::rstest;
    use serde_json::json;

    use super::*;
    use crate::integration::training::ExperimentTrainingExt;
    use crate::test_support::{MockRun, MockSession, mock_run};
    use crate::{ActivityEvent, ActivitySpec, ActivityStatus};

    const TRAINING: &str = "Pretraining";
    const EVALUATION: &str = "Holdout";
    const EPOCHS: usize = 2;
    const TESTS: usize = 2;
    const ITEMS: usize = 10;
    const RESUMED_AT: usize = 3;

    fn started(session: &MockSession, name: &str) -> ActivitySpec {
        session
            .started_activities()
            .into_iter()
            .find(|activity| activity.name == name)
            .unwrap()
    }

    fn epoch_of(session: &MockSession, split: &str) -> ActivitySpec {
        let split = started(session, split);
        session
            .started_activities()
            .into_iter()
            .find(|activity| Some(activity.id) == split.parent)
            .unwrap()
    }

    fn ending(session: &MockSession, name: &str) -> Option<ActivityStatus> {
        let id = started(session, name).id;
        session
            .activity_events()
            .into_iter()
            .find_map(|event| match event {
                ActivityEvent::Finished {
                    id: finished,
                    status,
                    ..
                } if finished == id => Some(status),
                _ => None,
            })
    }

    fn total(session: &MockSession, name: &str) -> Option<u64> {
        started(session, name).meter.and_then(|meter| meter.total)
    }

    #[rstest]
    fn the_training_activity_takes_the_name_of_its_logger(mock_run: MockRun) {
        let mut logger = mock_run.run.training_progress_logger().with_name(TRAINING);

        logger.start(EPOCHS, 1, None, None);

        assert_eq!(mock_run.session.started_activities()[0].name, TRAINING);
    }

    #[rstest]
    fn a_split_nests_under_its_epoch_under_the_training_under_its_scope(mock_run: MockRun) {
        let fold = mock_run.run.activity("Fold 1").start();
        let mut logger = fold.training_progress_logger().with_name(TRAINING);

        logger.start(EPOCHS, 1, None, None);
        logger.start_split("train", ITEMS);

        let training = started(&mock_run.session, TRAINING);
        let epoch = epoch_of(&mock_run.session, "train");
        assert_eq!(
            (training.parent, epoch.parent),
            (Some(fold.id()), Some(training.id))
        );
    }

    #[rstest]
    fn the_training_is_metered_by_its_epochs_and_a_split_by_its_items(mock_run: MockRun) {
        let mut logger = mock_run.run.training_progress_logger().with_name(TRAINING);

        logger.start(EPOCHS, 1, None, None);
        logger.start_split("train", ITEMS);

        assert_eq!(
            (
                total(&mock_run.session, TRAINING),
                total(&mock_run.session, "train")
            ),
            (Some(EPOCHS as u64), Some(ITEMS as u64))
        );
    }

    #[rstest]
    fn an_epoch_activity_numbers_its_epoch_from_where_the_training_started(mock_run: MockRun) {
        let mut logger = mock_run.run.training_progress_logger().with_name(TRAINING);

        logger.start(EPOCHS, RESUMED_AT, None, None);
        logger.start_split("train", ITEMS);

        assert_eq!(
            epoch_of(&mock_run.session, "train").attributes.get("epoch"),
            Some(&json!(RESUMED_AT))
        );
    }

    #[rstest]
    #[case::through_its_items(ITEMS, ActivityStatus::Success)]
    #[case::short_of_its_items(ITEMS - 1, ActivityStatus::Abandoned)]
    fn a_split_ends_by_how_far_its_meter_got(
        mock_run: MockRun,
        #[case] processed: usize,
        #[case] status: ActivityStatus,
    ) {
        let mut logger = mock_run.run.training_progress_logger().with_name(TRAINING);
        logger.start(EPOCHS, 1, None, None);
        logger.start_split("train", ITEMS);

        logger.update_split(processed);
        logger.end_split();

        assert_eq!(ending(&mock_run.session, "train"), Some(status));
    }

    #[rstest]
    #[case::every_epoch(EPOCHS, ActivityStatus::Success)]
    #[case::short_of_its_epochs(EPOCHS - 1, ActivityStatus::Abandoned)]
    fn the_training_ends_by_how_many_epochs_it_completed(
        mock_run: MockRun,
        #[case] completed: usize,
        #[case] status: ActivityStatus,
    ) {
        let mut logger = mock_run.run.training_progress_logger().with_name(TRAINING);
        logger.start(EPOCHS, 1, None, None);

        logger.update_epoch(completed);
        logger.end();

        assert_eq!(ending(&mock_run.session, TRAINING), Some(status));
    }

    #[rstest]
    fn a_test_nests_under_the_evaluation_under_its_scope(mock_run: MockRun) {
        let fold = mock_run.run.activity("Fold 1").start();
        let mut logger = fold.evaluation_progress_logger().with_name(EVALUATION);

        logger.start_global_progress(TESTS);
        logger.start_test("test", ITEMS);

        let evaluation = started(&mock_run.session, EVALUATION);
        let test = started(&mock_run.session, "test");
        assert_eq!(
            (evaluation.parent, test.parent),
            (Some(fold.id()), Some(evaluation.id))
        );
    }

    #[rstest]
    fn the_evaluation_is_metered_by_its_tests_and_a_test_by_its_items(mock_run: MockRun) {
        let mut logger = mock_run
            .run
            .evaluation_progress_logger()
            .with_name(EVALUATION);

        logger.start_global_progress(TESTS);
        logger.start_test("test", ITEMS);

        assert_eq!(
            (
                total(&mock_run.session, EVALUATION),
                total(&mock_run.session, "test")
            ),
            (Some(TESTS as u64), Some(ITEMS as u64))
        );
    }
}
