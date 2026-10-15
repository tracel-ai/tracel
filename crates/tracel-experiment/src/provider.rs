use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{ExperimentGlobalExt, ExperimentRun};

/// A backend that records experiments: it creates the run of each experiment started.
pub trait ExperimentProvider: Send + Sync + 'static {
    /// Creates the experiment `name`, which records `attributes`, and returns its run.
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError>;
}

/// Called with the run of each experiment started, once it exists.
type OnStart = Arc<dyn Fn(&ExperimentRun) + Send + Sync>;

/// The experiments of a backend, and how each experiment started from them starts: the attributes
/// and arguments it records, and the hooks called with its run.
///
/// [`run`](Self::run) runs code in a new experiment and ends the experiment by what the code
/// returns. [`start`](Self::start) starts one and returns its run, for code that ends it itself.
///
/// ```no_run
/// use tracel_experiment::{ExperimentRun, Experiments};
///
/// type BoxError = Box<dyn std::error::Error + Send + Sync>;
///
/// fn train(run: &ExperimentRun) -> Result<(), BoxError> {
///     run.log_info("training");
///     Ok(())
/// }
///
/// fn train_tracked(experiments: Experiments) -> Result<(), BoxError> {
///     experiments.attribute("dataset", "mnist")?.run("train", train)
/// }
/// ```
#[derive(Clone)]
pub struct Experiments {
    provider: Arc<dyn ExperimentProvider>,
    attributes: HashMap<String, Value>,
    arguments: Option<Value>,
    on_start: Vec<OnStart>,
}

impl Experiments {
    /// Experiments backed by `provider`, recording no attributes.
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self {
            provider,
            attributes: HashMap::new(),
            arguments: None,
            on_start: Vec::new(),
        }
    }

    /// Records `value`, as JSON, as the attribute `key` of each experiment started from the
    /// returned experiments.
    ///
    /// Fails when `value` does not serialize.
    pub fn attribute(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, ExperimentError> {
        let value = serde_json::to_value(value).map_err(|e| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to serialize experiment attribute",
                e,
            )
        })?;

        self.attributes.insert(key.into(), value);
        Ok(self)
    }

    /// Records `attributes` on each experiment started from the returned experiments.
    pub fn attributes(mut self, attributes: HashMap<String, Value>) -> Self {
        self.attributes.extend(attributes);
        self
    }

    /// Records `arguments`, JSON such as the configuration a run trains with, as the arguments of
    /// each experiment started from the returned experiments, when it starts.
    ///
    /// [`ExperimentRun::log_args`] replaces them: the arguments logged last are the run's.
    pub fn arguments(mut self, arguments: Value) -> Self {
        self.arguments = Some(arguments);
        self
    }

    /// Calls `hook` with the run of each experiment started from the returned experiments, once
    /// it exists and before [`start`](Self::start) returns it, after the hooks given before.
    pub fn on_start(mut self, hook: impl Fn(&ExperimentRun) + Send + Sync + 'static) -> Self {
        self.on_start.push(Arc::new(hook));
        self
    }

    /// Starts the experiment `name` and returns its run.
    ///
    /// The experiment records the attributes and arguments these experiments give it, and each
    /// [`on_start`](Self::on_start) hook is called with the run. The run ends as completed
    /// through [`ExperimentRun::finish`], and as failed through [`ExperimentRun::fail`] or when it
    /// is dropped before either. Unless the program has installed a `tracing` subscriber, this
    /// installs one that forwards events to experiments.
    pub fn start(&self, name: &str) -> Result<ExperimentRun, ExperimentError> {
        let _ = try_init_tracing_subscriber();

        let run = self
            .provider
            .create_experiment(name.to_string(), self.attributes.clone())?;
        if let Some(arguments) = &self.arguments {
            run.record_args(arguments.clone());
        }
        for hook in &self.on_start {
            hook(&run);
        }
        Ok(run)
    }

    /// Runs `f` in a new experiment `name`, as [`start`](Self::start) starts it, and ends the
    /// experiment by what `f` returns: completed when it returns `Ok`, and failed with the
    /// error's message when it returns `Err`, or with the panic's message when it panics.
    ///
    /// While `f` runs, the run is the current experiment, as
    /// [`ExperimentGlobalExt::in_scope`] makes it, so `tracing` events reach it, and a panic on any
    /// thread is logged to it, as [`ExperimentRun::capture_panics`] logs it. Fails without running
    /// `f` when the experiment does not start, and with the backend's error when it cannot be
    /// finished.
    pub fn run<T, E>(
        &self,
        name: &str,
        f: impl FnOnce(&ExperimentRun) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ExperimentError> + fmt::Display,
    {
        let run = self.start(name)?;
        let _panic_watch = run.capture_panics();
        match run.in_scope(|| f(&run)) {
            Ok(output) => {
                run.finish()?;
                Ok(output)
            }
            Err(error) => {
                let _ = run.fail(error.to_string());
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rstest::{fixture, rstest};
    use serde_json::json;

    use super::*;
    use crate::session::{Event, ExperimentCompletion};
    use crate::test_support::{MockSession, create_run_with_id};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    const REASON: &str = "diverged";

    #[derive(Default)]
    struct MockProvider {
        session: Arc<MockSession>,
        attributes: Mutex<Vec<HashMap<String, Value>>>,
    }

    impl ExperimentProvider for MockProvider {
        fn create_experiment(
            &self,
            _name: String,
            attributes: HashMap<String, Value>,
        ) -> Result<ExperimentRun, ExperimentError> {
            let mut created = self.attributes.lock().unwrap();
            created.push(attributes);
            Ok(create_run_with_id(
                created.len().to_string(),
                self.session.clone(),
            ))
        }
    }

    struct Fixture {
        provider: Arc<MockProvider>,
        experiments: Experiments,
    }

    impl Fixture {
        fn completions(&self) -> Vec<ExperimentCompletion> {
            self.provider.session.completions.lock().unwrap().clone()
        }

        fn arguments(&self) -> Vec<Value> {
            self.provider
                .session
                .events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Args(value) => Some(value.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    #[fixture]
    fn fixture() -> Fixture {
        let provider = Arc::new(MockProvider::default());
        Fixture {
            experiments: Experiments::new(provider.clone()),
            provider,
        }
    }

    #[rstest]
    #[case::ok(Ok(()), ExperimentCompletion::Success)]
    #[case::err(Err(REASON.into()), ExperimentCompletion::Failed(REASON.to_string()))]
    fn run_ends_the_experiment_by_what_its_code_returns(
        fixture: Fixture,
        #[case] returned: Result<(), BoxError>,
        #[case] completion: ExperimentCompletion,
    ) {
        let _ = fixture.experiments.run("train", |_run| returned);

        assert_eq!(fixture.completions(), [completion]);
    }

    #[rstest]
    fn a_panic_in_run_fails_the_experiment_with_its_message(fixture: Fixture) {
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fixture
                .experiments
                .run("train", |_run| -> Result<(), BoxError> {
                    panic!("loss is NaN")
                })
        }));

        assert!(panicked.is_err());
        match fixture.completions().as_slice() {
            [ExperimentCompletion::Failed(reason)] => {
                assert!(reason.ends_with("loss is NaN"), "{reason}");
            }
            other => panic!("unexpected completions: {other:?}"),
        }
    }

    #[rstest]
    fn run_makes_the_run_the_current_experiment(fixture: Fixture) {
        let current = fixture.experiments.run("train", |run| {
            let current = ExperimentRun::current().map(|current| current.id().clone());
            Ok::<_, BoxError>(current == Some(run.id().clone()))
        });

        assert!(current.unwrap());
    }

    #[test]
    fn run_runs_nothing_when_the_experiment_does_not_start() {
        struct Unreachable;

        impl ExperimentProvider for Unreachable {
            fn create_experiment(
                &self,
                _name: String,
                _attributes: HashMap<String, Value>,
            ) -> Result<ExperimentRun, ExperimentError> {
                Err(ExperimentError::new(
                    ExperimentErrorKind::Internal,
                    "the backend is unreachable",
                ))
            }
        }
        let mut ran = false;

        Experiments::new(Arc::new(Unreachable))
            .run("train", |_run| -> Result<(), BoxError> {
                ran = true;
                Ok(())
            })
            .unwrap_err();

        assert!(!ran);
    }

    #[rstest]
    fn a_started_experiment_records_the_attributes(fixture: Fixture) {
        let experiments = fixture
            .experiments
            .clone()
            .attribute("kind", "example")
            .unwrap()
            .attributes(HashMap::from([("seed".to_string(), json!(42))]));

        experiments.start("train").unwrap().finish().unwrap();

        assert_eq!(
            *fixture.provider.attributes.lock().unwrap(),
            [HashMap::from([
                ("kind".to_string(), json!("example")),
                ("seed".to_string(), json!(42)),
            ])]
        );
    }

    #[rstest]
    fn only_experiments_given_arguments_record_them_when_a_run_starts(fixture: Fixture) {
        let experiments = fixture.experiments.clone().arguments(json!({"epochs": 2}));

        experiments.start("train").unwrap().finish().unwrap();
        let recorded = fixture.arguments();
        fixture
            .experiments
            .start("train")
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(recorded, [json!({"epochs": 2})]);
        assert_eq!(fixture.arguments(), recorded);
    }

    #[rstest]
    fn arguments_logged_on_a_run_are_recorded_after_those_it_started_with(fixture: Fixture) {
        let experiments = fixture.experiments.clone().arguments(json!({"epochs": 2}));
        let run = experiments.start("train").unwrap();

        run.log_args(&json!({"epochs": 2, "fold": 1})).unwrap();

        assert_eq!(
            fixture.arguments(),
            [json!({"epochs": 2}), json!({"epochs": 2, "fold": 1})]
        );
    }

    #[rstest]
    fn each_hook_is_called_with_each_run_started(fixture: Fixture) {
        let called = Arc::new(Mutex::new(Vec::new()));
        let hook = |tag: &'static str| {
            let called = called.clone();
            move |run: &ExperimentRun| called.lock().unwrap().push((tag, run.id().clone()))
        };
        let experiments = fixture
            .experiments
            .clone()
            .on_start(hook("first"))
            .on_start(hook("second"));

        let first = experiments.start("train").unwrap();
        fixture
            .experiments
            .start("train")
            .unwrap()
            .finish()
            .unwrap();
        let second = experiments.start("train").unwrap();

        assert_eq!(
            *called.lock().unwrap(),
            [
                ("first", first.id().clone()),
                ("second", first.id().clone()),
                ("first", second.id().clone()),
                ("second", second.id().clone())
            ]
        );
    }
}
