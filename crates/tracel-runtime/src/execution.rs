//! Running a prepared job, as any transport does: the process adapter, a Station runner, or a
//! program's own.

use std::error::Error;

use serde_json::Value;

use crate::BoxError;
use crate::job::Ready;
use crate::panics::catch_panic;
use crate::scope;
use crate::stop::Stop;

/// A job given its input, ready to run once, as [`Jobs::prepare`](crate::Jobs::prepare) gives it.
///
/// It can be sent to the thread that runs it.
pub struct PreparedJob {
    name: String,
    input: Value,
    run: Ready,
}

/// The job `name`, given `input`, which `run` runs with.
pub fn prepared(name: String, input: Value, run: Ready) -> PreparedJob {
    PreparedJob { name, input, run }
}

impl PreparedJob {
    /// The name of the job.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The input the job runs with, as JSON: the input given, merged onto the job's default.
    pub fn input(&self) -> &Value {
        &self.input
    }

    /// Runs the job on this thread, launched as `execution` says, and returns how it ended.
    ///
    /// The job runs in a [`JobScope`](crate::JobScope) of its own, current on this thread while
    /// it runs: [`job_num`](crate::job_num) gives the number `execution` gives,
    /// [`job_input`](crate::job_input) the job's input, and
    /// [`stop_requested`](crate::stop_requested) and [`on_stop`](crate::on_stop) follow the
    /// execution's [`Stop`]. A panic in the job is a failure with the panic's message.
    pub fn run(self, execution: Execution) -> Outcome {
        let Execution { num, stop } = execution;
        let scope = scope::new(num, self.input, stop.clone());
        let result = {
            let _entered = scope::enter(scope);
            catch_panic(self.run)
        };
        Outcome {
            result,
            stop_requested: stop.is_requested(),
        }
    }
}

/// How one run of a job is launched: the number or id its transport gives it, and the [`Stop`]
/// that asks it to stop.
#[derive(Default)]
pub struct Execution {
    num: Option<String>,
    stop: Stop,
}

impl Execution {
    /// A run with no number, that nothing asks to stop.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives the job the number or id `num`, which [`job_num`](crate::job_num) gives in the job.
    pub fn with_num(mut self, num: impl Into<String>) -> Self {
        self.num = Some(num.into());
        self
    }

    /// Asks the job to stop once `stop` is requested, which the transport keeps a clone of to
    /// request.
    pub fn with_stop(mut self, stop: Stop) -> Self {
        self.stop = stop;
        self
    }
}

/// How a job ended, by what it returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobStatus {
    /// The job returned `Ok`.
    Completed,
    /// The job returned an error or panicked.
    Failed,
}

/// How a run of a job ended.
#[derive(Debug)]
pub struct Outcome {
    result: Result<(), BoxError>,
    stop_requested: bool,
}

impl Outcome {
    /// Whether the job completed or failed, by what it returned, whether or not it was asked to
    /// stop.
    pub fn status(&self) -> JobStatus {
        match self.result {
            Ok(()) => JobStatus::Completed,
            Err(_) => JobStatus::Failed,
        }
    }

    /// Why the job failed: the error it returned, or its panic's message; `None` when it
    /// completed.
    pub fn error(&self) -> Option<&(dyn Error + Send + Sync + 'static)> {
        self.result.as_ref().err().map(AsRef::as_ref)
    }

    /// Whether the job was asked to stop before it ended.
    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }

    /// What the job returned: `Ok` when it completed, the error it failed with otherwise.
    pub fn into_result(self) -> Result<(), BoxError> {
        self.result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier, Mutex};

    use serde_json::json;

    use super::*;
    use crate::{
        Job, Jobs, PrepareError, job_input, job_num, job_running, on_stop, stop_requested,
    };

    /// `train`, which takes a number of epochs, three by default, and fails on zero; and `panic`,
    /// which panics.
    fn jobs() -> Jobs {
        Jobs::new()
            .job(
                "train",
                Job::new(|epochs: u32| match epochs {
                    0 => Err::<(), BoxError>("no epochs".into()),
                    _ => Ok(()),
                })
                .with_default(3),
            )
            .job("panic", |()| -> Result<(), BoxError> { panic!("kaboom") })
    }

    #[test]
    fn a_job_is_prepared_with_its_input_merged_onto_its_default() {
        let jobs = Jobs::new().job(
            "train",
            Job::new(|_: Value| Ok::<_, BoxError>(()))
                .with_default(json!({"epochs": 3, "lr": 0.1})),
        );

        let prepared = jobs.prepare("train", json!({"epochs": 5})).unwrap();

        assert_eq!(prepared.name(), "train");
        assert_eq!(prepared.input(), &json!({"epochs": 5, "lr": 0.1}));
    }

    #[test]
    fn an_unknown_job_or_an_input_that_does_not_decode_is_not_prepared() {
        let jobs = jobs();

        let unknown = jobs.prepare("infer", Value::Null).err().unwrap();
        let invalid = jobs.prepare("train", json!("ten")).err().unwrap();

        assert!(
            matches!(
                &unknown,
                PrepareError::UnknownJob { name, available }
                    if name == "infer" && available == &["panic", "train"]
            ),
            "{unknown:?}"
        );
        assert_eq!(unknown.to_string(), "unknown job 'infer'");
        assert!(
            matches!(invalid, PrepareError::InvalidInput(_)),
            "{invalid:?}"
        );
        assert!(
            invalid.to_string().starts_with("invalid input: "),
            "{invalid}"
        );
    }

    #[test]
    fn a_job_completes_or_fails_by_what_it_returns_and_a_panic_fails_it() {
        let run =
            |name: &str, input: Value| jobs().prepare(name, input).unwrap().run(Execution::new());

        let completed = run("train", Value::Null);
        let failed = run("train", json!(0));
        let panicked = run("panic", Value::Null);

        assert_eq!(completed.status(), JobStatus::Completed);
        assert!(completed.error().is_none());
        assert!(!completed.stop_requested());
        assert!(completed.into_result().is_ok());
        assert_eq!(failed.status(), JobStatus::Failed);
        assert_eq!(failed.error().unwrap().to_string(), "no epochs");
        assert_eq!(panicked.status(), JobStatus::Failed);
        assert_eq!(
            panicked.into_result().unwrap_err().to_string(),
            "the job panicked: kaboom"
        );
    }

    #[test]
    fn the_job_reads_its_number_and_input_from_its_scope_while_it_runs() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let jobs = Jobs::new().job(
            "train",
            Job::new({
                let seen = seen.clone();
                move |_: Value| {
                    seen.lock()
                        .unwrap()
                        .push((job_running(), job_num(), job_input()));
                    Ok::<_, BoxError>(())
                }
            })
            .with_default(json!({"epochs": 3})),
        );

        let numbered = jobs.prepare("train", json!({"lr": 0.1})).unwrap();
        numbered.run(Execution::new().with_num("42"));
        jobs.prepare("train", Value::Null)
            .unwrap()
            .run(Execution::new());

        assert_eq!(
            *seen.lock().unwrap(),
            [
                (
                    true,
                    Some("42".to_string()),
                    Some(json!({"epochs": 3, "lr": 0.1}))
                ),
                (true, None, Some(json!({"epochs": 3}))),
            ]
        );
        assert!(!job_running());
    }

    #[test]
    fn the_stop_the_transport_requests_runs_what_the_job_registered_and_is_seen_in_it() {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let stop = Stop::new();
        let jobs = Jobs::new().job("train", {
            let (ran, stop) = (ran.clone(), stop.clone());
            move |()| {
                let on_stop_ran = ran.clone();
                on_stop(move || on_stop_ran.lock().unwrap().push("on stop"));
                ran.lock().unwrap().push("before");
                stop.request();
                let seen = if stop_requested() {
                    "stop requested"
                } else {
                    "not requested"
                };
                ran.lock().unwrap().push(seen);
                Err::<(), BoxError>("interrupted".into())
            }
        });

        let outcome = jobs
            .prepare("train", Value::Null)
            .unwrap()
            .run(Execution::new().with_stop(stop.clone()));

        assert_eq!(outcome.status(), JobStatus::Failed);
        assert!(outcome.stop_requested());
        assert_eq!(
            *ran.lock().unwrap(),
            ["before", "on stop", "stop requested"]
        );
    }

    #[test]
    fn runs_at_once_on_two_threads_each_see_their_own_execution() {
        let both = Arc::new(Barrier::new(2));
        let jobs = Arc::new(Jobs::new().job("train", {
            let both = both.clone();
            move |epochs: u32| {
                both.wait();
                match (job_num(), job_input(), stop_requested()) {
                    (Some(num), Some(input), stopped)
                        if num == epochs.to_string()
                            && input == json!(epochs)
                            && stopped == (epochs == 1) =>
                    {
                        Ok(())
                    }
                    seen => Err::<(), BoxError>(format!("job {epochs} saw {seen:?}").into()),
                }
            }
        }));
        let run = |epochs: u32, stop: Stop| {
            let prepared = jobs.prepare("train", json!(epochs)).unwrap();
            let execution = Execution::new()
                .with_num(epochs.to_string())
                .with_stop(stop);
            std::thread::spawn(move || prepared.run(execution))
        };
        let stopped = Stop::new();
        stopped.request();

        let one = run(1, stopped);
        let two = run(2, Stop::new());

        let (one, two) = (one.join().unwrap(), two.join().unwrap());
        assert!(one.error().is_none(), "{one:?}");
        assert!(two.error().is_none(), "{two:?}");
        assert!(one.stop_requested());
        assert!(!two.stop_requested());
    }

    #[test]
    fn jobs_and_runs_can_move_to_the_threads_that_run_them() {
        fn send_and_sync<T: Send + Sync>() {}
        fn send<T: Send>() {}

        send_and_sync::<Jobs>();
        send_and_sync::<Stop>();
        send::<PreparedJob>();
        send::<Execution>();
        send::<Outcome>();
    }
}
