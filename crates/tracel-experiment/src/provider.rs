use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{CancelToken, ExperimentGlobalExt, ExperimentRun};

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
/// it records, the token that asks it to stop, and the hooks called with its run.
///
/// [`run`](Self::run) runs code in a new experiment and ends the experiment by what the code
/// returns. [`start`](Self::start) starts one and returns its run, for code that ends it itself.
/// A clone, and the experiments [`attribute`](Self::attribute), [`attributes`](Self::attributes)
/// and [`on_start`](Self::on_start) return, share the [`cancel_token`](Self::cancel_token).
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
    cancel_token: CancelToken,
    on_start: Vec<OnStart>,
}

impl Experiments {
    /// Experiments backed by `provider`, recording no attributes.
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self {
            provider,
            attributes: HashMap::new(),
            cancel_token: CancelToken::new(),
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

    /// Calls `hook` with the run of each experiment started from the returned experiments, once
    /// it exists and before [`start`](Self::start) returns it, after the hooks given before.
    pub fn on_start(mut self, hook: impl Fn(&ExperimentRun) + Send + Sync + 'static) -> Self {
        self.on_start.push(Arc::new(hook));
        self
    }

    /// The token that asks the experiments started from these experiments to stop.
    ///
    /// Cancelling it cancels the [cancel token](ExperimentRun::cancel_token) of each run started
    /// from them, before or after, so each logs once that a stop was requested, and they start
    /// no more: [`start`](Self::start) then fails with [`ExperimentErrorKind::Cancelled`].
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel_token.clone()
    }

    /// Starts the experiment `name` and returns its run.
    ///
    /// The experiment records the attributes these experiments give it, its run's cancel token is
    /// cancelled with their [`cancel_token`](Self::cancel_token), and each
    /// [`on_start`](Self::on_start) hook is called with the run. The run ends as completed
    /// through [`ExperimentRun::finish`], and as failed through [`ExperimentRun::fail`] or when it
    /// is dropped before either. Unless the program has installed a `tracing` subscriber, this
    /// installs one that forwards events to experiments.
    ///
    /// Once the experiments' cancel token is cancelled, this fails with
    /// [`ExperimentErrorKind::Cancelled`] without creating the experiment.
    pub fn start(&self, name: &str) -> Result<ExperimentRun, ExperimentError> {
        if self.cancel_token.is_cancelled() {
            return Err(ExperimentError::new(
                ExperimentErrorKind::Cancelled,
                format!("experiment '{name}' was not started: a stop was requested"),
            ));
        }

        let _ = try_init_tracing_subscriber();

        let run = self
            .provider
            .create_experiment(name.to_string(), self.attributes.clone())?;
        self.cancel_token.link(run.cancel_token());
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
        // Worker-thread panics (a kernel compiler, a data loader) land in the
        // run's log even when they never unwind the run itself.
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
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::json;

    use super::*;
    use crate::session::{Event, ExperimentCompletion};
    use crate::test_support::{MockSession, create_run_with_id};
    use crate::{ExperimentId, LogLevel, STOP_REQUESTED};

    type BoxError = Box<dyn std::error::Error + Send + Sync>;

    /// Creates runs numbered 7 on one session, and keeps the attributes each was created with.
    struct MockProvider {
        session: Arc<MockSession>,
        attributes: Arc<Mutex<Vec<HashMap<String, Value>>>>,
    }

    impl ExperimentProvider for MockProvider {
        fn create_experiment(
            &self,
            _name: String,
            attributes: HashMap<String, Value>,
        ) -> Result<ExperimentRun, ExperimentError> {
            self.attributes.lock().unwrap().push(attributes);
            Ok(create_run_with_id("7", self.session.clone()))
        }
    }

    struct Fixture {
        session: Arc<MockSession>,
        attributes: Arc<Mutex<Vec<HashMap<String, Value>>>>,
        experiments: Experiments,
    }

    impl Fixture {
        fn new() -> Self {
            let session = Arc::new(MockSession::default());
            let attributes = Arc::default();
            let experiments = Experiments::new(Arc::new(MockProvider {
                session: session.clone(),
                attributes: Arc::clone(&attributes),
            }));
            Self {
                session,
                attributes,
                experiments,
            }
        }

        fn completions(&self) -> Vec<ExperimentCompletion> {
            self.session.completions.lock().unwrap().clone()
        }

        /// The warnings the runs logged.
        fn warnings(&self) -> Vec<String> {
            self.session
                .events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|event| match event {
                    Event::Log { record, .. } if record.level == LogLevel::Warn => {
                        Some(record.message.clone())
                    }
                    _ => None,
                })
                .collect()
        }
    }

    #[test]
    fn run_ends_the_experiment_by_what_its_code_returns() {
        let fixture = Fixture::new();

        let output = fixture
            .experiments
            .run("train", |_run| Ok::<_, BoxError>(3));
        let failed = fixture
            .experiments
            .run("train", |_run| Err::<(), BoxError>("diverged".into()));

        assert_eq!(output.unwrap(), 3);
        assert_eq!(failed.unwrap_err().to_string(), "diverged");
        assert_eq!(
            fixture.completions(),
            [
                ExperimentCompletion::Success,
                ExperimentCompletion::Failed("diverged".to_string())
            ]
        );
    }

    #[test]
    fn a_panic_in_run_fails_the_experiment_with_its_message() {
        let fixture = Fixture::new();

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
                assert!(reason.contains("loss is NaN"), "{reason}");
            }
            other => panic!("unexpected completions: {other:?}"),
        }
    }

    #[test]
    fn run_makes_the_run_the_current_experiment() {
        let fixture = Fixture::new();

        let current = fixture.experiments.run("train", |run| {
            let current = ExperimentRun::current().map(|current| current.id().clone());
            Ok::<_, BoxError>(current == Some(run.id().clone()))
        });

        assert!(current.unwrap());
        assert!(ExperimentRun::current().is_none());
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
        let ran = AtomicBool::new(false);

        let error = Experiments::new(Arc::new(Unreachable))
            .run("train", |_run| -> Result<(), BoxError> {
                ran.store(true, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err();

        assert_eq!(error.to_string(), "the backend is unreachable");
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn a_started_experiment_records_the_attributes() {
        let fixture = Fixture::new();
        let experiments = fixture
            .experiments
            .clone()
            .attribute("kind", "example")
            .unwrap()
            .attributes(HashMap::from([("seed".to_string(), json!(42))]));

        experiments.start("train").unwrap().finish().unwrap();

        assert_eq!(
            *fixture.attributes.lock().unwrap(),
            [HashMap::from([
                ("kind".to_string(), json!("example")),
                ("seed".to_string(), json!(42)),
            ])]
        );
    }

    #[test]
    fn each_hook_is_called_with_each_run_started() {
        let fixture = Fixture::new();
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

        experiments.start("train").unwrap().finish().unwrap();
        fixture
            .experiments
            .start("train")
            .unwrap()
            .finish()
            .unwrap();

        assert_eq!(
            *called.lock().unwrap(),
            [
                ("first", ExperimentId::from("7")),
                ("second", ExperimentId::from("7"))
            ]
        );
    }

    #[test]
    fn cancelling_the_experiments_asks_each_run_started_from_them_to_stop() {
        let fixture = Fixture::new();
        let experiments = fixture.experiments.clone().attribute("kind", "example");

        let run = experiments.unwrap().start("train").unwrap();
        fixture.experiments.cancel_token().cancel();

        assert!(run.cancel_token().is_cancelled());
        assert_eq!(fixture.warnings(), [STOP_REQUESTED]);
    }

    #[test]
    fn no_experiment_starts_once_the_experiments_are_cancelled() {
        let fixture = Fixture::new();
        let called = Arc::new(AtomicBool::new(false));
        let experiments = fixture.experiments.clone().on_start({
            let called = called.clone();
            move |_run| called.store(true, Ordering::SeqCst)
        });
        experiments.cancel_token().cancel();

        let started = experiments.start("train");

        assert!(matches!(
            started,
            Err(ExperimentError {
                kind: ExperimentErrorKind::Cancelled,
                ..
            })
        ));
        assert!(fixture.attributes.lock().unwrap().is_empty());
        assert!(!called.load(Ordering::SeqCst));
    }
}
