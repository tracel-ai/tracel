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
    use std::sync::{Arc, Barrier, Mutex};

    use serde_json::json;

    use super::*;
    use crate::{Job, Jobs, job_input, job_num, job_running};

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
    fn runs_at_once_on_two_threads_each_see_their_own_execution() {
        let both = Arc::new(Barrier::new(2));
        let jobs = Arc::new(Jobs::new().job("train", {
            let both = both.clone();
            move |epochs: u32| {
                both.wait();
                match (job_num(), job_input()) {
                    (Some(num), Some(input))
                        if num == epochs.to_string() && input == json!(epochs) =>
                    {
                        Ok(())
                    }
                    seen => Err::<(), BoxError>(format!("job {epochs} saw {seen:?}").into()),
                }
            }
        }));
        let run = |epochs: u32| {
            let prepared = jobs.prepare("train", json!(epochs)).unwrap();
            let execution = Execution::new().with_num(epochs.to_string());
            std::thread::spawn(move || prepared.run(execution))
        };

        let one = run(1);
        let two = run(2);

        let (one, two) = (one.join().unwrap(), two.join().unwrap());
        assert!(one.error().is_none(), "{one:?}");
        assert!(two.error().is_none(), "{two:?}");
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
