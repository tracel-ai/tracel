//! The runner loop: SSE reader, single-job executor, reconnect with backoff.

use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tracel_experiment::CancelToken;
use uuid::Uuid;

use crate::infrastructure::protocol::{
    DispatchedJob, FinishJob, FinishStatus, RegisterRunner, RunnerEvent,
};
use crate::infrastructure::{ClientError, StationRunnerClient};
use crate::job::RunnerJob;

pub(crate) type JobTable = HashMap<String, Box<dyn RunnerJob>>;

/// Reports job outcomes back to the station. Split out so the loop is testable with a fake.
pub(crate) trait FinishSink: Send + Sync + 'static {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob);
}

impl FinishSink for StationRunnerClient {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob) {
        // A rejected finish is expected after a disconnect: the station has already failed the
        // job and this session no longer owns it.
        if let Err(e) = StationRunnerClient::finish_job(self, job_id, finish) {
            tracing::warn!(error = %e, job_id = %job_id, "Failed to report job outcome");
        }
    }
}

struct Dispatch {
    runner_id: Uuid,
    job: DispatchedJob,
    cancel: CancelToken,
}

type CurrentJob = Arc<(Mutex<Option<(Uuid, CancelToken)>>, Condvar)>;

/// Executes dispatched jobs one at a time on a dedicated thread, so the event stream keeps being
/// read (for cancels and liveness) while user code runs.
pub(crate) struct Executor {
    sender: mpsc::Sender<Dispatch>,
    current: CurrentJob,
}

impl Executor {
    pub fn spawn(jobs: Arc<JobTable>, sink: Arc<dyn FinishSink>) -> Self {
        let (sender, receiver) = mpsc::channel::<Dispatch>();
        let current: CurrentJob = Arc::default();

        let worker_current = current.clone();
        std::thread::spawn(move || {
            for Dispatch {
                runner_id,
                job,
                cancel,
            } in receiver
            {
                let (status, reason) = execute(&jobs, &job, cancel);
                sink.finish_job(
                    job.id,
                    &FinishJob {
                        runner_id,
                        status,
                        reason,
                    },
                );
                let (slot, finished) = &*worker_current;
                *slot.lock().unwrap() = None;
                finished.notify_all();
            }
        });

        Self { sender, current }
    }

    /// Hand a job to the executor. The current-job slot is set before the hand-off so a cancel
    /// arriving right after dispatch always finds the token.
    pub fn dispatch(&self, runner_id: Uuid, job: DispatchedJob) {
        let cancel = CancelToken::new();
        {
            let (slot, _) = &*self.current;
            let mut slot = slot.lock().unwrap();
            if slot.is_some() {
                tracing::warn!(job_id = %job.id, "Job dispatched while another is still running");
            }
            *slot = Some((job.id, cancel.clone()));
        }
        if self
            .sender
            .send(Dispatch {
                runner_id,
                job,
                cancel,
            })
            .is_err()
        {
            tracing::error!("Executor thread is gone; dropping dispatched job");
        }
    }

    pub fn cancel(&self, job_id: Uuid) {
        let (slot, _) = &*self.current;
        if let Some((current_id, cancel)) = slot.lock().unwrap().as_ref()
            && *current_id == job_id
        {
            cancel.cancel();
        }
    }

    pub fn cancel_current(&self) {
        let (slot, _) = &*self.current;
        if let Some((_, cancel)) = slot.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }

    /// Block until the in-flight job (if any) has finished and reported.
    pub fn wait_idle(&self) {
        let (slot, finished) = &*self.current;
        let mut slot = slot.lock().unwrap();
        while slot.is_some() {
            slot = finished.wait(slot).unwrap();
        }
    }
}

fn execute(
    jobs: &JobTable,
    job: &DispatchedJob,
    cancel: CancelToken,
) -> (FinishStatus, Option<String>) {
    let Some(runner_job) = jobs.get(&job.job_name) else {
        // Unreachable under the station's strict policy; defend anyway.
        return (
            FinishStatus::Failed,
            Some(format!("unknown job '{}'", job.job_name)),
        );
    };
    tracing::info!(job_id = %job.id, job_name = %job.job_name, "Running job");
    let result = catch_unwind(AssertUnwindSafe(|| {
        runner_job.run(&job.input, cancel.clone())
    }));
    match result {
        Err(panic) => (
            FinishStatus::Failed,
            Some(format!("job panicked: {}", panic_message(panic.as_ref()))),
        ),
        Ok(_) if cancel.is_cancelled() => (FinishStatus::Cancelled, None),
        Ok(Ok(())) => (FinishStatus::Completed, None),
        Ok(Err(e)) => (FinishStatus::Failed, Some(e.to_string())),
    }
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message
    } else {
        "unknown panic"
    }
}

pub(crate) enum StreamOutcome {
    /// The stream ended before the `registered` event; nothing was served.
    NeverRegistered,
    /// The session registered and served until the stream ended.
    Served,
}

/// Drive one session: expect the leading `registered` event, then route job dispatches to the
/// executor and cancels to the in-flight token, until the stream ends.
pub(crate) fn serve_stream(
    mut events: impl Iterator<Item = Result<RunnerEvent, ClientError>>,
    executor: &Executor,
) -> StreamOutcome {
    let runner_id = match events.next() {
        Some(Ok(RunnerEvent::Registered { runner_id })) => runner_id,
        Some(Ok(event)) => {
            tracing::warn!(?event, "Expected a registered event first");
            return StreamOutcome::NeverRegistered;
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "Event stream failed before registration");
            return StreamOutcome::NeverRegistered;
        }
        None => {
            tracing::warn!("Event stream closed before registration");
            return StreamOutcome::NeverRegistered;
        }
    };
    tracing::info!(runner_id = %runner_id, "Registered with station");

    for event in events {
        match event {
            Ok(RunnerEvent::Job(job)) => executor.dispatch(runner_id, job),
            Ok(RunnerEvent::Cancel { job_id }) => executor.cancel(job_id),
            Ok(RunnerEvent::Registered { .. }) => {
                tracing::warn!("Unexpected registered event mid-stream");
            }
            Err(e) => {
                tracing::warn!(error = %e, "Event stream error");
                break;
            }
        }
    }
    StreamOutcome::Served
}

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

struct Backoff {
    delay: Duration,
}

impl Backoff {
    fn new() -> Self {
        Self { delay: MIN_BACKOFF }
    }

    fn next(&mut self) -> Duration {
        let delay = self.delay;
        self.delay = (self.delay * 2).min(MAX_BACKOFF);
        delay
    }

    fn reset(&mut self) {
        self.delay = MIN_BACKOFF;
    }
}

/// Serve sessions forever: (re)connect, serve until the stream drops, drain, back off, repeat.
pub(crate) fn serve_forever(
    client: StationRunnerClient,
    register: RegisterRunner,
    executor: Executor,
) -> ! {
    let mut backoff = Backoff::new();
    loop {
        match client.open_events(&register) {
            Ok(stream) => {
                if let StreamOutcome::Served = serve_stream(stream, &executor) {
                    backoff.reset();
                }
                // The station has already failed the in-flight job on disconnect: stop it
                // locally, and never re-register while user code still runs — a fresh session
                // would get a concurrent dispatch.
                executor.cancel_current();
                executor.wait_idle();
            }
            Err(e) => tracing::warn!(error = %e, "Failed to connect to station"),
        }
        let delay = backoff.next();
        tracing::info!(delay_secs = delay.as_secs(), "Reconnecting to station");
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use serde_json::Value;

    use super::*;
    use crate::error::BoxError;
    use crate::job::JobDefinition;

    #[allow(clippy::type_complexity)]
    type RecordedFinish = (Uuid, Uuid, FinishStatus, Option<String>);

    #[derive(Default)]
    struct RecordingSink {
        finishes: Mutex<Vec<RecordedFinish>>,
    }

    impl FinishSink for RecordingSink {
        fn finish_job(&self, job_id: Uuid, finish: &FinishJob) {
            self.finishes.lock().unwrap().push((
                job_id,
                finish.runner_id,
                finish.status,
                finish.reason.clone(),
            ));
        }
    }

    enum FakeBehaviour {
        Succeed,
        Fail(&'static str),
        Panic(&'static str),
        WaitForCancel,
    }

    struct FakeJob {
        name: &'static str,
        behaviour: FakeBehaviour,
        ran: Arc<AtomicBool>,
    }

    impl FakeJob {
        fn new(name: &'static str, behaviour: FakeBehaviour) -> Self {
            Self {
                name,
                behaviour,
                ran: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl RunnerJob for FakeJob {
        fn definition(&self) -> JobDefinition {
            JobDefinition {
                name: self.name.to_string(),
                description: None,
                input_schema: None,
                input_example: None,
            }
        }

        fn run(&self, _input: &Value, cancel: CancelToken) -> Result<(), BoxError> {
            self.ran.store(true, Ordering::SeqCst);
            match &self.behaviour {
                FakeBehaviour::Succeed => Ok(()),
                FakeBehaviour::Fail(reason) => Err((*reason).into()),
                FakeBehaviour::Panic(message) => panic!("{message}"),
                FakeBehaviour::WaitForCancel => {
                    while !cancel.is_cancelled() {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Ok(())
                }
            }
        }
    }

    struct Setup {
        executor: Executor,
        sink: Arc<RecordingSink>,
        runner_id: Uuid,
    }

    fn setup(jobs: Vec<FakeJob>) -> Setup {
        let table: JobTable = jobs
            .into_iter()
            .map(|job| (job.name.to_string(), Box::new(job) as Box<dyn RunnerJob>))
            .collect();
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::spawn(Arc::new(table), sink.clone());
        Setup {
            executor,
            sink,
            runner_id: Uuid::new_v4(),
        }
    }

    fn dispatched(job_name: &str) -> DispatchedJob {
        DispatchedJob {
            id: Uuid::new_v4(),
            job_name: job_name.to_string(),
            input: serde_json::json!({}),
        }
    }

    fn events(
        runner_id: Uuid,
        rest: Vec<RunnerEvent>,
    ) -> impl Iterator<Item = Result<RunnerEvent, ClientError>> {
        std::iter::once(RunnerEvent::Registered { runner_id })
            .chain(rest)
            .map(Ok)
    }

    #[test]
    fn given_job_event_when_serving_then_completion_reported() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![FakeJob::new("train", FakeBehaviour::Succeed)]);
        let job = dispatched("train");

        serve_stream(
            events(runner_id, vec![RunnerEvent::Job(job.clone())]),
            &executor,
        );
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(
            finishes.as_slice(),
            &[(job.id, runner_id, FinishStatus::Completed, None)]
        );
    }

    #[test]
    fn given_failing_job_then_failure_reason_reported() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![FakeJob::new("train", FakeBehaviour::Fail("boom"))]);

        serve_stream(
            events(runner_id, vec![RunnerEvent::Job(dispatched("train"))]),
            &executor,
        );
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("boom"));
    }

    #[test]
    fn given_panicking_job_then_failure_reported_with_panic_message() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![FakeJob::new("train", FakeBehaviour::Panic("kaboom"))]);

        serve_stream(
            events(runner_id, vec![RunnerEvent::Job(dispatched("train"))]),
            &executor,
        );
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("job panicked: kaboom"));
    }

    #[test]
    fn given_unknown_job_name_then_failure_reported() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![]);

        serve_stream(
            events(runner_id, vec![RunnerEvent::Job(dispatched("mystery"))]),
            &executor,
        );
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("unknown job 'mystery'"));
    }

    #[test]
    fn given_cancel_for_running_job_then_cancellation_reported() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![FakeJob::new("train", FakeBehaviour::WaitForCancel)]);
        let job = dispatched("train");

        serve_stream(
            events(
                runner_id,
                vec![
                    RunnerEvent::Job(job.clone()),
                    RunnerEvent::Cancel { job_id: job.id },
                ],
            ),
            &executor,
        );
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(
            finishes.as_slice(),
            &[(job.id, runner_id, FinishStatus::Cancelled, None)]
        );
    }

    #[test]
    fn given_cancel_for_other_job_then_running_job_unaffected() {
        let Setup {
            executor,
            sink,
            runner_id,
        } = setup(vec![FakeJob::new("train", FakeBehaviour::WaitForCancel)]);
        let job = dispatched("train");

        serve_stream(
            events(
                runner_id,
                vec![
                    RunnerEvent::Job(job.clone()),
                    RunnerEvent::Cancel {
                        job_id: Uuid::new_v4(),
                    },
                ],
            ),
            &executor,
        );
        // The stream is over (station gone): the loop stops the in-flight job before draining.
        executor.cancel_current();
        executor.wait_idle();

        let finishes = sink.finishes.lock().unwrap();
        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0].2, FinishStatus::Cancelled);
    }

    #[test]
    fn given_stream_without_registered_first_then_nothing_served() {
        let Setup { executor, sink, .. } =
            setup(vec![FakeJob::new("train", FakeBehaviour::Succeed)]);

        let outcome = serve_stream(
            vec![Ok(RunnerEvent::Job(dispatched("train")))].into_iter(),
            &executor,
        );
        executor.wait_idle();

        assert!(matches!(outcome, StreamOutcome::NeverRegistered));
        assert!(sink.finishes.lock().unwrap().is_empty());
    }
}
