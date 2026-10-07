use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::integration::tracing::try_init_tracing_subscriber;
use crate::{CancelToken, ExperimentRun};

/// Set while a program writes its job definitions instead of running a job.
const TRACEL_DESCRIBE: &str = "TRACEL_DESCRIBE";
/// Names the file a run writes its run report to.
const TRACEL_REPORT_FILE: &str = "TRACEL_REPORT_FILE";
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

/// Entry point for building experiment jobs against a backend.
#[derive(Clone)]
pub struct Experiments {
    provider: Arc<dyn ExperimentProvider>,
}

impl Experiments {
    // TODO: Add settings here (e.g., an Experiments builder).
    /// Create experiments backed by the given provider.
    pub fn new(provider: Arc<dyn ExperimentProvider>) -> Self {
        Self { provider }
    }

    pub fn create<I, O>(
        &self,
        name: &str,
        f: impl ExperimentFn<I, O> + 'static,
    ) -> ExperimentJob<I, O> {
        ExperimentJob::new(self.provider.clone(), name.to_string(), f)
    }
}

pub struct ExperimentJob<I, O> {
    provider: Arc<dyn ExperimentProvider>,
    name: String,
    description: Option<String>,
    attributes: HashMap<String, Value>,
    f: Arc<dyn ExperimentFn<I, O>>,
}

impl<I, O> Clone for ExperimentJob<I, O> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            attributes: self.attributes.clone(),
            f: self.f.clone(),
        }
    }
}

impl<I, O> ExperimentJob<I, O> {
    fn new<F>(provider: Arc<dyn ExperimentProvider>, name: String, f: F) -> Self
    where
        F: ExperimentFn<I, O> + 'static,
    {
        Self {
            provider,
            name,
            description: None,
            attributes: HashMap::new(),
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

    pub fn attributes(mut self, attrs: HashMap<String, Value>) -> Self {
        self.attributes.extend(attrs);
        self
    }

    /// Runs the job with `input`, recording one experiment with `input` as its arguments.
    ///
    /// While `TRACEL_DESCRIBE` is set, the program is describing its jobs: this returns an
    /// [`ExperimentErrorKind::Describing`] error without creating an experiment. When
    /// `TRACEL_REPORT_FILE` names a path, the run report is written there when the experiment is
    /// created and again when the run ends. When `TRACEL_JOB_NUM` is set, the experiment records
    /// it as the attribute `tracel.job_num`.
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
        self.run_with(input, arguments, CancelToken::new())
    }

    /// Runs the job like [`run`](Self::run), recording `arguments` as the experiment's arguments
    /// instead of serializing `input`, and cancelling the run when `cancel_token` is cancelled.
    ///
    /// `arguments` is typically the JSON `input` was decoded from; `null` records none. A run
    /// whose token is cancelled ends as cancelled, and one cancelled before it starts creates no
    /// experiment and returns an [`ExperimentErrorKind::Cancelled`] error.
    pub fn run_with(
        &self,
        input: I,
        arguments: Value,
        cancel_token: CancelToken,
    ) -> Result<O, Box<dyn Error + Send + Sync>> {
        self.run_with_vars(
            |name| std::env::var_os(name),
            input,
            arguments,
            cancel_token,
        )
    }

    /// [`run_with`](Self::run_with) with the environment variables `lookup` gives.
    fn run_with_vars(
        &self,
        lookup: impl Fn(&str) -> Option<OsString>,
        input: I,
        arguments: Value,
        cancel_token: CancelToken,
    ) -> Result<O, Box<dyn Error + Send + Sync>> {
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        if var(TRACEL_DESCRIBE).is_some() {
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

        let _ = try_init_tracing_subscriber();

        let mut attributes = self.attributes.clone();
        if let Some(job_num) = job_num(var(TRACEL_JOB_NUM)) {
            attributes.insert(JOB_NUM_ATTRIBUTE.to_string(), job_num);
        }
        let mut experiment = self
            .provider
            .create_experiment(self.name.clone(), attributes)?;
        cancel_token.link(experiment.cancel_token());
        if !arguments.is_null() {
            experiment.record_args(arguments);
        }
        if let Some(path) = var(TRACEL_REPORT_FILE)
            && let Err(error) = experiment.report_to(&self.name, PathBuf::from(path))
        {
            let _ = experiment.fail(error.to_string());
            return Err(error.into());
        }

        let handle = experiment.handle();
        // Worker-thread panics (a kernel compiler, a data loader) land in the
        // run's log even when they never unwind the run itself.
        let _panic_watch = experiment.capture_panics();
        let result = handle.in_scope(|| self.f.call(&experiment, input));

        match result {
            Ok(output) => {
                if experiment.cancel_token().is_cancelled() {
                    // Dropping without an explicit completion finalizes the run as cancelled.
                    drop(experiment);
                } else {
                    experiment.finish()?;
                }
                Ok(output)
            }
            Err(e) if experiment.cancel_token().is_cancelled() => {
                drop(experiment);
                Err(e)
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = experiment.fail(msg);
                Err(e)
            }
        }
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
    use std::path::Path;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::json;

    use super::*;
    use crate::session::{Event, ExperimentCompletion};
    use crate::test_support::{MockSession, create_run_with_id};

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
    fn vars(vars: &[(&'static str, &Path)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: Vec<(&'static str, OsString)> = vars
            .iter()
            .map(|(name, value)| (*name, value.as_os_str().to_owned()))
            .collect();
        move |name| {
            vars.iter()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| value.clone())
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
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
    fn given_run_cancelled_through_control_when_running_then_complete_cancelled() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |run: &ExperimentRun, _input: ()| {
                // A remote backend would cancel the run's own control plane; simulate that here.
                run.cancel_token().cancel();
                Ok(())
            });

        job.run(()).unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Cancelled]);
    }

    #[test]
    fn given_run_error_when_cancel_started_then_complete_cancelled() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |run: &ExperimentRun, _input: ()| {
                run.cancel_token().cancel();
                Err::<(), _>("interrupted".into())
            });

        let result = job.run(());

        assert!(result.is_err());
        assert_eq!(fixture.completions(), [ExperimentCompletion::Cancelled]);
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
        job.run_with(json!("ignored"), json!({"epochs": 3}), CancelToken::new())
            .unwrap();
        job.run_with(json!("ignored"), Value::Null, CancelToken::new())
            .unwrap();

        assert_eq!(
            fixture.arguments(),
            [json!({"epochs": 2}), json!({"epochs": 3})]
        );
    }

    #[test]
    fn cancelling_the_token_given_cancels_the_run() {
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

        job.run_with((), Value::Null, cancel).unwrap();

        assert_eq!(fixture.completions(), [ExperimentCompletion::Cancelled]);
    }

    #[test]
    fn a_run_cancelled_before_it_starts_creates_no_experiment() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()));
        let cancel = CancelToken::new();
        cancel.cancel();

        let error = job.run_with((), Value::Null, cancel).unwrap_err();

        let error = error.downcast::<ExperimentError>().unwrap();
        assert_eq!(error.kind, ExperimentErrorKind::Cancelled);
        assert!(fixture.attributes.lock().unwrap().is_empty());
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

        let error = job
            .run_with_vars(
                vars(&[(TRACEL_DESCRIBE, Path::new("jobs.json"))]),
                (),
                Value::Null,
                CancelToken::new(),
            )
            .unwrap_err();

        let error = error.downcast::<ExperimentError>().unwrap();
        assert_eq!(error.kind, ExperimentErrorKind::Describing);
        assert!(!called.load(Ordering::SeqCst));
        assert!(fixture.completions().is_empty());
    }

    #[test]
    fn an_empty_tracel_describe_runs_the_experiment() {
        let fixture = Fixture::new();
        let job = fixture
            .experiments
            .create("job", |_run: &ExperimentRun, _input: ()| Ok(()));

        job.run_with_vars(
            vars(&[(TRACEL_DESCRIBE, Path::new(""))]),
            (),
            Value::Null,
            CancelToken::new(),
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

        job.run_with_vars(
            vars(&[(TRACEL_JOB_NUM, Path::new("12"))]),
            (),
            Value::Null,
            CancelToken::new(),
        )
        .unwrap();
        job.run_with_vars(
            vars(&[(TRACEL_JOB_NUM, Path::new(""))]),
            (),
            Value::Null,
            CancelToken::new(),
        )
        .unwrap();

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
    fn the_report_file_follows_the_run_from_running_to_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let fixture = Fixture::new();
        let job = fixture.experiments.create("mnist", {
            let path = path.clone();
            move |_run: &ExperimentRun, fail: bool| -> Result<(), Box<dyn Error + Send + Sync>> {
                let report = read_json(&path);
                assert_eq!(report["status"], "running");
                assert_eq!(report["finished_at"], Value::Null);
                if fail { Err("diverged".into()) } else { Ok(()) }
            }
        });

        job.run_with_vars(
            vars(&[(TRACEL_REPORT_FILE, &path)]),
            false,
            Value::Null,
            CancelToken::new(),
        )
        .unwrap();
        let completed = read_json(&path);
        job.run_with_vars(
            vars(&[(TRACEL_REPORT_FILE, &path)]),
            true,
            Value::Null,
            CancelToken::new(),
        )
        .unwrap_err();
        let failed = read_json(&path);

        assert_eq!(completed["job"], "mnist");
        assert_eq!(completed["experiment"], json!({"num": 7, "url": null}));
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["error"], Value::Null);
        assert!(completed["finished_at"].is_string());
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"], "diverged");
    }

    #[test]
    fn a_cancelled_run_is_reported_cancelled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let fixture = Fixture::new();
        let cancel = CancelToken::new();
        let job = fixture.experiments.create("job", {
            let cancel = cancel.clone();
            move |_run: &ExperimentRun, _input: ()| {
                cancel.cancel();
                Err::<(), _>("interrupted".into())
            }
        });

        job.run_with_vars(
            vars(&[(TRACEL_REPORT_FILE, &path)]),
            (),
            Value::Null,
            cancel,
        )
        .unwrap_err();

        let report = read_json(&path);
        assert_eq!(report["status"], "cancelled");
        assert_eq!(report["error"], Value::Null);
    }

    #[test]
    fn a_panicking_run_is_reported_failed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let fixture = Fixture::new();
        let job = fixture.experiments.create(
            "job",
            |_run: &ExperimentRun, _input: ()| -> Result<(), Box<dyn Error + Send + Sync>> {
                panic!("kernel exploded")
            },
        );

        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            job.run_with_vars(
                vars(&[(TRACEL_REPORT_FILE, &path)]),
                (),
                Value::Null,
                CancelToken::new(),
            )
        }));

        assert!(outcome.is_err());
        let report = read_json(&path);
        assert_eq!(report["status"], "failed");
        assert!(
            report["error"]
                .as_str()
                .is_some_and(|error| error.contains("kernel exploded")),
            "{report}"
        );
    }

    #[test]
    fn a_report_file_that_cannot_be_written_fails_the_run_before_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");
        let fixture = Fixture::new();
        let called = Arc::new(AtomicBool::new(false));
        let job = fixture.experiments.create("job", {
            let called = called.clone();
            move |_run: &ExperimentRun, _input: ()| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            }
        });

        let error = job
            .run_with_vars(
                vars(&[(TRACEL_REPORT_FILE, &path)]),
                (),
                Value::Null,
                CancelToken::new(),
            )
            .unwrap_err();

        assert!(error.to_string().contains("run report"), "{error}");
        assert!(!called.load(Ordering::SeqCst));
        assert!(matches!(
            fixture.completions().as_slice(),
            [ExperimentCompletion::Failed(_)]
        ));
    }
}
