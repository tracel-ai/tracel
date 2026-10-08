use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tracel_job::TRACEL_DESCRIBE;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{CancelToken, ExperimentRun};

/// The number of the job a launcher runs, recorded as the experiment attribute
/// [`JOB_NUM_ATTRIBUTE`].
const TRACEL_JOB_NUM: &str = "TRACEL_JOB_NUM";
/// The experiment attribute that links an experiment to the job that ran it.
const JOB_NUM_ATTRIBUTE: &str = "tracel.job_num";

pub trait ExperimentProvider: Send + Sync + 'static {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError>;
}

pub trait ExperimentFn<I, O>: Send + Sync {
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>>;
}

impl<I, O, F> ExperimentFn<I, O> for F
where
    F: Fn(&ExperimentRun, I) -> Result<O, Box<dyn Error + Send + Sync>> + Send + Sync,
{
    fn call(&self, run: &ExperimentRun, input: I) -> Result<O, Box<dyn Error + Send + Sync>> {
        (self)(run, input)
    }
}

/// The experiments of a backend, with the attributes each experiment started from it records.
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
}

impl Experiments {
    /// Experiments backed by `provider`, recording no attributes.
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self {
            provider,
            attributes: HashMap::new(),
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

    /// Starts the experiment `name` and returns its run.
    ///
    /// The run ends as completed through [`ExperimentRun::finish`], and as failed through
    /// [`ExperimentRun::fail`] or when it is dropped before either. Unless the program has
    /// installed a `tracing` subscriber, this installs one that forwards events to experiments.
    pub fn start(&self, name: &str) -> Result<ExperimentRun, ExperimentError> {
        self.start_with(name, self.attributes.clone())
    }

    /// [`start`](Self::start), recording `attributes`.
    fn start_with(
        &self,
        name: &str,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError> {
        let _ = try_init_tracing_subscriber();
        self.provider
            .create_experiment(name.to_string(), attributes)
    }

    /// Runs `f` in a new experiment `name`, as [`start`](Self::start) starts it, and ends the
    /// experiment by what `f` returns: completed when it returns `Ok`, and failed with the
    /// error's message when it returns `Err`, or with the panic's message when it panics.
    ///
    /// While `f` runs, the run is the current experiment, as
    /// [`ExperimentGlobalExt::in_scope`](crate::ExperimentGlobalExt::in_scope) makes it, so
    /// `tracing` events reach it, and a panic on any thread is logged to it, as
    /// [`ExperimentRun::capture_panics`] logs it. Fails without running `f` when the experiment
    /// does not start, and with the backend's error when it cannot be finished.
    pub fn run<T, E>(
        &self,
        name: &str,
        f: impl FnOnce(&ExperimentRun) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<ExperimentError> + fmt::Display,
    {
        end_by(self.start(name)?, f)
    }

    pub fn create<I, O>(
        &self,
        name: &str,
        f: impl ExperimentFn<I, O> + 'static,
    ) -> ExperimentJob<I, O> {
        ExperimentJob::new(self.clone(), name.to_string(), f)
    }
}

/// Runs `f` with `run`, as [`Experiments::run`] does, and ends `run` by what `f` returns.
fn end_by<T, E>(run: ExperimentRun, f: impl FnOnce(&ExperimentRun) -> Result<T, E>) -> Result<T, E>
where
    E: From<ExperimentError> + fmt::Display,
{
    let handle = run.handle();
    // Worker-thread panics (a kernel compiler, a data loader) land in the
    // run's log even when they never unwind the run itself.
    let _panic_watch = run.capture_panics();
    match handle.in_scope(|| f(&run)) {
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

pub struct ExperimentJob<I, O> {
    experiments: Experiments,
    name: String,
    description: Option<String>,
    f: Arc<dyn ExperimentFn<I, O>>,
}

impl<I, O> Clone for ExperimentJob<I, O> {
    fn clone(&self) -> Self {
        Self {
            experiments: self.experiments.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            f: self.f.clone(),
        }
    }
}

impl<I, O> ExperimentJob<I, O> {
    fn new<F>(experiments: Experiments, name: String, f: F) -> Self
    where
        F: ExperimentFn<I, O> + 'static,
    {
        Self {
            experiments,
            name,
            description: None,
            f: Arc::new(f),
        }
    }

    /// The job's name, used to select it from a runner.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The job's description, as runners list it.
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Sets the job's description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn attribute(
        mut self,
        key: impl Into<String>,
        value: impl Serialize,
    ) -> Result<Self, ExperimentError> {
        self.experiments = self.experiments.attribute(key, value)?;
        Ok(self)
    }

    pub fn attributes(mut self, attrs: HashMap<String, Value>) -> Self {
        self.experiments = self.experiments.attributes(attrs);
        self
    }

    /// Runs the job with `input`, recording one experiment with `input` as its arguments.
    ///
    /// While `TRACEL_DESCRIBE` is set, the program is describing its jobs: this returns an
    /// [`ExperimentErrorKind::Describing`] error without creating an experiment. When
    /// `TRACEL_JOB_NUM` is set, the experiment records it as the attribute `tracel.job_num`.
    pub fn run(&self, input: I) -> Result<O, Box<dyn Error + Send + Sync>>
    where
        I: Serialize,
    {
        let arguments = serde_json::to_value(&input).map_err(|error| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to serialize the experiment input",
                error,
            )
        })?;
        self.run_with(input, arguments, CancelToken::new(), |_| {})
    }

    /// Runs the job like [`run`](Self::run), recording `arguments` as the experiment's arguments
    /// instead of serializing `input`, cancelling the run's cancel token when `cancel_token` is
    /// cancelled, and calling `on_created` with the run once the experiment exists, before the
    /// job's function runs.
    ///
    /// `arguments` is typically the JSON `input` was decoded from; `null` records none. A run whose
    /// token is cancelled still ends as completed or failed, by what the job's function returns.
    /// One cancelled before it starts creates no experiment and returns an
    /// [`ExperimentErrorKind::Cancelled`] error.
    pub fn run_with(
        &self,
        input: I,
        arguments: Value,
        cancel_token: CancelToken,
        on_created: impl FnOnce(&ExperimentRun),
    ) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run_with_vars(
            |name| std::env::var_os(name),
            input,
            arguments,
            cancel_token,
            on_created,
        )
    }

    /// [`run_with`](Self::run_with) with the environment variables `lookup` gives.
    fn run_with_vars(
        &self,
        lookup: impl Fn(&str) -> Option<OsString>,
        input: I,
        arguments: Value,
        cancel_token: CancelToken,
        on_created: impl FnOnce(&ExperimentRun),
    ) -> Result<O, Box<dyn Error + Send + Sync>> {
        if lookup(TRACEL_DESCRIBE).is_some_and(|value| !value.is_empty()) {
            return Err(ExperimentError::new(
                ExperimentErrorKind::Describing,
                format!(
                    "experiment '{}' does not run while TRACEL_DESCRIBE is set",
                    self.name
                ),
            )
            .into());
        }
        if cancel_token.is_cancelled() {
            return Err(ExperimentError::new(
                ExperimentErrorKind::Cancelled,
                format!("experiment '{}' was cancelled before it started", self.name),
            )
            .into());
        }

        let mut attributes = self.experiments.attributes.clone();
        if let Some(num) = job_num(lookup(TRACEL_JOB_NUM).filter(|value| !value.is_empty())) {
            attributes.insert(JOB_NUM_ATTRIBUTE.to_string(), num);
        }
        let experiment = self.experiments.start_with(&self.name, attributes)?;
        cancel_token.link(experiment.cancel_token());
        if !arguments.is_null() {
            experiment.record_args(arguments);
        }
        on_created(&experiment);

        end_by(experiment, |run| self.f.call(run, input))
    }
}

/// The attribute value `TRACEL_JOB_NUM` gives: a number when it is one, its text otherwise.
fn job_num(value: Option<OsString>) -> Option<Value> {
    let value = value?.to_string_lossy().into_owned();
    Some(match value.parse::<u64>() {
        Ok(num) => Value::from(num),
        Err(_) => Value::from(value),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::json;

    use super::*;
    use crate::session::{Event, ExperimentCompletion};
    use crate::test_support::{MockSession, create_run_with_id};
    use crate::{ExperimentGlobalExt, ExperimentId};

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

        fn arguments(&self) -> Vec<Value> {
            self.session
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

    /// The variables a launcher sets: `vars` and nothing else.
    fn vars(vars: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars = vars.to_vec();
        move |name| {
            vars.iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    /// An `on_created` hook that keeps the ID of each run it is called with.
    fn created() -> (
        impl Fn(&ExperimentRun) + Clone,
        Arc<Mutex<Vec<ExperimentId>>>,
    ) {
        let created = Arc::new(Mutex::new(Vec::new()));
        let hook = {
            let created = created.clone();
            move |run: &ExperimentRun| created.lock().unwrap().push(run.id().clone())
        };
        (hook, created)
    }

    type BoxError = Box<dyn Error + Send + Sync>;

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
    fn given_run_when_completing_then_finalizes_with_success() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()));

        job.run(()).unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Success]);
    }

    #[test]
    fn given_run_cancelled_through_control_when_running_then_complete_success() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |run: &ExperimentRun, _input: ()| {
                // A remote backend would cancel the run's own control plane; simulate that here.
                run.cancel_token().cancel();
                Ok(())
            });

        job.run(()).unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Success]);
    }

    #[test]
    fn given_run_error_when_cancel_started_then_complete_failed() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |run: &ExperimentRun, _input: ()| {
                run.cancel_token().cancel();
                Err::<(), _>("interrupted".into())
            });

        let result = job.run(());

        assert!(result.is_err());
        assert_eq!(
            fixture.completions(),
            [ExperimentCompletion::Failed("interrupted".to_string())]
        );
    }

    #[test]
    fn given_run_error_when_running_then_complete_failed() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| {
                Err::<(), _>("boom".into())
            });

        let result = job.run(());

        assert!(result.is_err());
        assert_eq!(
            fixture.completions(),
            [ExperimentCompletion::Failed("boom".to_string())]
        );
    }

    #[test]
    fn a_run_records_its_input_as_the_experiment_arguments() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: Value| Ok(()));

        job.run(json!({"epochs": 2})).unwrap();
        job.run_with(
            json!("ignored"),
            json!({"epochs": 3}),
            CancelToken::new(),
            |_| {},
        )
        .unwrap();
        job.run_with(json!("ignored"), Value::Null, CancelToken::new(), |_| {})
            .unwrap();

        assert_eq!(
            fixture.arguments(),
            [json!({"epochs": 2}), json!({"epochs": 3})]
        );
    }

    #[test]
    fn cancelling_the_token_given_cancels_the_run_token() {
        let fixture = Fixture::new();
        let cancel = CancelToken::new();
        let job = fixture.experiments.create("job", {
            let cancel = cancel.clone();
            move |run: &ExperimentRun, _input: ()| {
                cancel.cancel();
                assert!(run.cancel_token().is_cancelled());
                Ok(())
            }
        });

        job.run_with((), Value::Null, cancel, |_| {}).unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Success]);
    }

    #[test]
    fn a_run_cancelled_before_it_starts_creates_no_experiment() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()));
        let cancel = CancelToken::new();
        cancel.cancel();

        let (hook, created) = created();

        let error = job.run_with((), Value::Null, cancel, hook).unwrap_err();

        let error = error.downcast::<ExperimentError>().unwrap();
        assert_eq!(error.kind, ExperimentErrorKind::Cancelled);
        assert!(fixture.attributes.lock().unwrap().is_empty());
        assert!(created.lock().unwrap().is_empty());
    }

    #[test]
    fn a_run_while_describing_creates_no_experiment() {
        let fixture = Fixture::new();
        let called = Arc::new(AtomicBool::new(false));
        let job = fixture.experiments.create("job", {
            let called = called.clone();
            move |_run: &ExperimentRun, _input: ()| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        let (hook, created) = created();

        let error = job
            .run_with_vars(
                vars(&[(TRACEL_DESCRIBE, "jobs.json")]),
                (),
                Value::Null,
                CancelToken::new(),
                hook,
            )
            .unwrap_err();

        let error = error.downcast::<ExperimentError>().unwrap();
        assert_eq!(error.kind, ExperimentErrorKind::Describing);
        assert!(!called.load(Ordering::SeqCst));
        assert!(fixture.completions().is_empty());
        assert!(created.lock().unwrap().is_empty());
    }

    #[test]
    fn an_empty_tracel_describe_runs_the_experiment() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()));

        job.run_with_vars(
            vars(&[(TRACEL_DESCRIBE, "")]),
            (),
            Value::Null,
            CancelToken::new(),
            |_| {},
        )
        .unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Success]);
    }

    #[test]
    fn tracel_job_num_is_recorded_as_a_number_when_it_is_one() {
        assert_eq!(job_num(Some("42".into())), Some(json!(42)));
        assert_eq!(job_num(Some("job-42".into())), Some(json!("job-42")));
        assert_eq!(job_num(None), None);
    }

    #[test]
    fn a_launched_run_links_its_experiment_to_the_job() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()))
            .attribute("kind", "example")
            .unwrap();

        for job_num in ["12", ""] {
            job.run_with_vars(
                vars(&[(TRACEL_JOB_NUM, job_num)]),
                (),
                Value::Null,
                CancelToken::new(),
                |_| {},
            )
            .unwrap();
        }

        let attributes = fixture.attributes.lock().unwrap();
        assert_eq!(
            attributes[0],
            HashMap::from([
                ("kind".to_string(), json!("example")),
                (JOB_NUM_ATTRIBUTE.to_string(), json!(12)),
            ])
        );
        assert_eq!(
            attributes[1],
            HashMap::from([("kind".to_string(), json!("example"))])
        );
    }

    #[test]
    fn on_created_is_called_with_the_run_before_its_function_runs() {
        let fixture = Fixture::new();
        let (hook, created) = created();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let job = fixture.experiments.create("job", {
            let (created, seen) = (created.clone(), seen.clone());
            move |run: &ExperimentRun, fail: bool| -> Result<(), Box<dyn Error + Send + Sync>> {
                seen.lock()
                    .unwrap()
                    .push(created.lock().unwrap().last() == Some(run.id()));
                if fail { Err("diverged".into()) } else { Ok(()) }
            }
        });

        job.run_with(false, Value::Null, CancelToken::new(), hook.clone())
            .unwrap();
        job.run_with(true, Value::Null, CancelToken::new(), hook)
            .unwrap_err();

        assert_eq!(*seen.lock().unwrap(), [true, true]);
        assert_eq!(
            *created.lock().unwrap(),
            [ExperimentId::from("7"), ExperimentId::from("7")]
        );
    }
}
