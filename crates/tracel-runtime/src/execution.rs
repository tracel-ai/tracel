//! Running a prepared job, as any transport does: the process adapter, a Station runner, or a
//! program's own.

use std::error::Error;

use serde_json::Value;

use crate::BoxError;
use crate::job::Ready;
use crate::panics::catch_panic;
use crate::scope;

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
    /// it runs: [`job_num`](crate::job_num) gives the number `execution` gives, and
    /// [`job_input`](crate::job_input) the job's input. A panic in the job is a failure with the
    /// panic's message.
    pub fn run(self, execution: Execution) -> Outcome {
        let _entered = scope::enter(scope::new(execution.num, self.input));
        Outcome {
            result: catch_panic(self.run),
        }
    }
}

/// How one run of a job is launched: the number or id its transport gives it.
#[derive(Default)]
pub struct Execution {
    num: Option<String>,
}

impl Execution {
    /// A run with no number.
    pub fn new() -> Self {
        Self::default()
    }

    /// Gives the job the number or id `num`, which [`job_num`](crate::job_num) gives in the job.
    pub fn with_num(mut self, num: impl Into<String>) -> Self {
        self.num = Some(num.into());
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
}

impl Outcome {
    /// Whether the job completed or failed, by what it returned.
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

    /// What the job returned: `Ok` when it completed, the error it failed with otherwise.
    pub fn into_result(self) -> Result<(), BoxError> {
        self.result
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use rstest::rstest;
    use serde_json::json;

    use super::*;
    use crate::{Job, Jobs, job_input, job_num};

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

    fn reading<T>(read: fn() -> T) -> (Jobs, Arc<OnceLock<T>>)
    where
        T: Send + Sync + 'static,
    {
        let seen = Arc::new(OnceLock::new());
        let jobs = Jobs::new().job("train", {
            let seen = seen.clone();
            move |_: Value| {
                let _ = seen.set(read());
                Ok::<_, BoxError>(())
            }
        });
        (jobs, seen)
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
        assert!(completed.into_result().is_ok());
        assert_eq!(failed.status(), JobStatus::Failed);
        assert_eq!(failed.error().unwrap().to_string(), "no epochs");
        assert_eq!(panicked.status(), JobStatus::Failed);
    }

    #[rstest]
    #[case::numbered(Execution::new().with_num("42"), Some("42"))]
    #[case::unnumbered(Execution::new(), None)]
    fn the_job_reads_the_number_its_execution_gives(
        #[case] execution: Execution,
        #[case] num: Option<&str>,
    ) {
        let (jobs, seen) = reading(job_num);

        jobs.prepare("train", Value::Null).unwrap().run(execution);

        assert_eq!(seen.get(), Some(&num.map(str::to_string)));
    }

    #[test]
    fn the_job_reads_the_input_it_runs_with() {
        let (jobs, seen) = reading(job_input);
        let prepared = jobs.prepare("train", json!({"lr": 0.1})).unwrap();
        let input = prepared.input().clone();

        prepared.run(Execution::new());

        assert_eq!(seen.get(), Some(&Some(input)));
    }

    #[test]
    fn jobs_and_runs_can_move_to_the_threads_that_run_them() {
        fn send_and_sync<T: Send + Sync>() {}
        fn send<T: Send>() {}

        send_and_sync::<Jobs>();
        send::<PreparedJob>();
        send::<Execution>();
        send::<Outcome>();
    }
}
