//! The runner loop: SSE reader, an executor running each dispatched job on a thread of its own,
//! reconnect with backoff.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tracing::{info, warn};
use uuid::Uuid;

use crate::station::error::RunnerError;
use crate::station::infrastructure::StationRunnerClient;
use crate::station::infrastructure::protocol::{
    DispatchedJob, FinishJob, FinishStatus, RegisterRunner, RunnerEvent,
};
use crate::{Execution, Jobs};

/// Reports job outcomes back to the station. Split out so the loop is testable with a fake.
pub trait FinishSink: Send + Sync + 'static {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob);
}

impl FinishSink for StationRunnerClient {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob) {
        // A rejected finish is expected after a disconnect: the station has already failed the
        // job and this session no longer owns it.
        if let Err(e) = StationRunnerClient::finish_job(self, job_id, finish) {
            warn!(%job_id, error = %e, "failed to report the outcome of the job to the station");
        }
    }
}

/// In-flight job count, paired with a condvar so [`Executor::wait_idle`] can block until it drains.
type InFlight = Arc<(Mutex<usize>, Condvar)>;

/// Runs each dispatched job on a thread of its own, in a scope of its own, so the event stream
/// keeps being read (for liveness) while user code runs, and jobs the station dispatches at once
/// run at once.
pub struct Executor {
    jobs: Arc<Jobs>,
    sink: Arc<dyn FinishSink>,
    in_flight: InFlight,
}

impl Executor {
    pub fn new(jobs: Arc<Jobs>, sink: Arc<dyn FinishSink>) -> Self {
        Self {
            jobs,
            sink,
            in_flight: Arc::default(),
        }
    }

    /// Runs `job` on a thread of its own, and reports how it ended to the station once it ends.
    pub fn dispatch(&self, runner_id: Uuid, job: DispatchedJob) {
        *self.in_flight.0.lock().unwrap() += 1;
        let (jobs, sink, in_flight) =
            (self.jobs.clone(), self.sink.clone(), self.in_flight.clone());
        let job_id = job.id;
        let spawned = std::thread::Builder::new()
            .name(format!("job-{job_id}"))
            .spawn(move || {
                let (status, reason) = execute(&jobs, &job);
                sink.finish_job(
                    job.id,
                    &FinishJob {
                        runner_id,
                        status,
                        reason,
                    },
                );
                finish_in_flight(&in_flight);
            });
        if let Err(error) = spawned {
            let reason = format!("the job could not start: {error}");
            self.sink.finish_job(
                job_id,
                &FinishJob {
                    runner_id,
                    status: FinishStatus::Failed,
                    reason: Some(reason),
                },
            );
            finish_in_flight(&self.in_flight);
        }
    }

    /// Block until every in-flight job has finished and reported.
    pub fn wait_idle(&self) {
        let (count, finished) = &*self.in_flight;
        let mut count = count.lock().unwrap();
        while *count > 0 {
            count = finished.wait(count).unwrap();
        }
    }
}

fn finish_in_flight(in_flight: &InFlight) {
    let (count, finished) = &**in_flight;
    *count.lock().unwrap() -= 1;
    finished.notify_all();
}

/// Runs `job` and returns how it ended: completed or failed by what it returned, with its error
/// when it failed.
fn execute(jobs: &Jobs, job: &DispatchedJob) -> (FinishStatus, Option<String>) {
    let prepared = match jobs.prepare(&job.job_name, job.input.clone()) {
        Ok(prepared) => prepared,
        // An unknown job is unreachable under the station's strict policy; defend anyway.
        Err(error) => return (FinishStatus::Failed, Some(error.to_string())),
    };
    info!(job = %job.job_name, job_id = %job.id, "running the job");
    let outcome = prepared.run(Execution::new().with_num(job.id.to_string()));
    match outcome.into_result() {
        Ok(()) => (FinishStatus::Completed, None),
        Err(error) => (FinishStatus::Failed, Some(error.to_string())),
    }
}

pub enum StreamOutcome {
    /// The stream ended before the `registered` event; nothing was served.
    NeverRegistered,
    /// The session registered and served until the stream ended.
    Served,
}

/// Drive one session: expect the leading `registered` event, then route job dispatches to the
/// executor until the stream ends.
pub fn serve_stream(
    mut events: impl Iterator<Item = RunnerEvent>,
    executor: &Executor,
) -> StreamOutcome {
    let runner_id = match events.next() {
        Some(RunnerEvent::Registered { runner_id }) => runner_id,
        Some(event) => {
            warn!(?event, "expected a registered event first");
            return StreamOutcome::NeverRegistered;
        }
        None => {
            warn!("the event stream closed before registration");
            return StreamOutcome::NeverRegistered;
        }
    };
    info!(%runner_id, "registered with the station");

    for event in events {
        match event {
            RunnerEvent::Job(job) => executor.dispatch(runner_id, job),
            RunnerEvent::Cancel { job_id } => {
                info!(%job_id, "the station cancelled the job; it runs to completion");
            }
            RunnerEvent::Registered { .. } => {
                warn!("unexpected registered event mid-stream");
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

/// Serve sessions until a fatal registration rejection: (re)connect, serve until the stream drops,
/// drain, back off, repeat. Transient connection failures are retried; a permanent rejection stops.
pub fn serve_forever(
    client: StationRunnerClient,
    register: RegisterRunner,
    executor: Executor,
) -> RunnerError {
    let mut backoff = Backoff::new();
    loop {
        match client.open_events(&register) {
            Ok(stream) => {
                if let StreamOutcome::Served = serve_stream(stream, &executor) {
                    backoff.reset();
                }
                // Never re-register while user code still runs: the station failed those jobs when
                // the stream dropped, and a fresh session would get new ones. Drain them first.
                executor.wait_idle();
            }
            Err(e) if e.is_permanent() => return RunnerError::Registration(e.to_string()),
            Err(e) => warn!(error = %e, "failed to connect to the station"),
        }
        let delay = backoff.next();
        info!(delay_secs = delay.as_secs(), "reconnecting to the station");
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{BoxError, job_input, job_num};

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

    /// `train` succeeds, `fail` fails, `panic` panics, `typed` takes a number, and `num` keeps the
    /// job number it is given.
    fn jobs(nums: Arc<Mutex<Vec<Option<String>>>>) -> Jobs {
        Jobs::new()
            .job("train", |_: Value| Ok::<_, BoxError>(()))
            .job("fail", |_: Value| Err::<(), BoxError>("boom".into()))
            .job("panic", |_: Value| -> Result<(), BoxError> {
                panic!("kaboom")
            })
            .job("typed", |_: u32| Ok::<_, BoxError>(()))
            .job("num", move |_: Value| {
                nums.lock().unwrap().push(job_num());
                Ok::<_, BoxError>(())
            })
    }

    struct Setup {
        executor: Executor,
        sink: Arc<RecordingSink>,
        runner_id: Uuid,
        nums: Arc<Mutex<Vec<Option<String>>>>,
    }

    fn setup() -> Setup {
        let nums = Arc::default();
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::new(Arc::new(jobs(Arc::clone(&nums))), sink.clone());
        Setup {
            executor,
            sink,
            runner_id: Uuid::new_v4(),
            nums,
        }
    }

    fn dispatched(job_name: &str) -> DispatchedJob {
        DispatchedJob {
            id: Uuid::new_v4(),
            job_name: job_name.to_string(),
            input: json!({}),
        }
    }

    fn events(runner_id: Uuid, rest: Vec<RunnerEvent>) -> impl Iterator<Item = RunnerEvent> {
        std::iter::once(RunnerEvent::Registered { runner_id }).chain(rest)
    }

    /// Serves `jobs`, dispatched in order, and returns the finishes reported.
    fn serve(setup: &Setup, jobs: Vec<DispatchedJob>) -> Vec<RecordedFinish> {
        let rest = jobs.into_iter().map(RunnerEvent::Job).collect();
        serve_stream(events(setup.runner_id, rest), &setup.executor);
        setup.executor.wait_idle();
        setup.sink.finishes.lock().unwrap().clone()
    }

    #[test]
    fn given_job_event_when_serving_then_completion_reported() {
        let setup = setup();
        let job = dispatched("train");

        let finishes = serve(&setup, vec![job.clone()]);

        assert_eq!(
            finishes,
            [(job.id, setup.runner_id, FinishStatus::Completed, None)]
        );
    }

    #[test]
    fn given_failing_job_then_failure_reason_reported() {
        let finishes = serve(&setup(), vec![dispatched("fail")]);

        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("boom"));
    }

    #[test]
    fn given_input_the_job_rejects_then_failure_reported_as_invalid_input() {
        let finishes = serve(&setup(), vec![dispatched("typed")]);

        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(
            finishes[0].3.as_deref(),
            Some("invalid input: invalid type: map, expected u32")
        );
    }

    #[test]
    fn given_panicking_job_then_failure_reported_with_panic_message() {
        let finishes = serve(&setup(), vec![dispatched("panic")]);

        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("the job panicked: kaboom"));
    }

    #[test]
    fn given_unknown_job_name_then_failure_reported() {
        let finishes = serve(&setup(), vec![dispatched("mystery")]);

        assert_eq!(finishes[0].2, FinishStatus::Failed);
        assert_eq!(finishes[0].3.as_deref(), Some("unknown job 'mystery'"));
    }

    #[test]
    fn a_running_job_is_numbered_with_its_station_job_id() {
        let setup = setup();
        let job = dispatched("num");

        serve(&setup, vec![job.clone()]);

        assert_eq!(*setup.nums.lock().unwrap(), [Some(job.id.to_string())]);
    }

    #[test]
    fn jobs_dispatched_at_once_run_at_once_each_in_its_own_scope() {
        let started = Arc::new((Mutex::new(0), Condvar::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let jobs = Jobs::new().job("wait", {
            let (started, seen) = (started.clone(), seen.clone());
            move |_: Value| {
                let (count, both) = &*started;
                *count.lock().unwrap() += 1;
                both.notify_all();
                let (count, _) = both
                    .wait_timeout_while(count.lock().unwrap(), Duration::from_secs(10), |count| {
                        *count < 2
                    })
                    .unwrap();
                seen.lock()
                    .unwrap()
                    .push((job_num(), job_input(), *count == 2));
                Ok::<_, BoxError>(())
            }
        });
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::new(Arc::new(jobs), sink.clone());
        let with_input = |n: u32| DispatchedJob {
            input: json!({ "n": n }),
            ..dispatched("wait")
        };
        let (first, second) = (with_input(1), with_input(2));

        serve_stream(
            events(
                Uuid::new_v4(),
                vec![
                    RunnerEvent::Job(first.clone()),
                    RunnerEvent::Job(second.clone()),
                ],
            ),
            &executor,
        );
        executor.wait_idle();

        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        for (job, n) in [(&first, 1), (&second, 2)] {
            assert!(
                seen.contains(&(Some(job.id.to_string()), Some(json!({ "n": n })), true)),
                "{seen:?}"
            );
        }
        assert_eq!(sink.finishes.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_cancelled_job_runs_to_completion_and_the_stream_goes_on() {
        let setup = setup();
        let (cancelled, next) = (dispatched("train"), dispatched("train"));

        serve_stream(
            events(
                setup.runner_id,
                vec![
                    RunnerEvent::Job(cancelled.clone()),
                    RunnerEvent::Cancel {
                        job_id: cancelled.id,
                    },
                    RunnerEvent::Job(next.clone()),
                ],
            ),
            &setup.executor,
        );
        setup.executor.wait_idle();

        let finishes = setup.sink.finishes.lock().unwrap().clone();
        assert_eq!(finishes.len(), 2);
        for job in [&cancelled, &next] {
            assert!(
                finishes.contains(&(job.id, setup.runner_id, FinishStatus::Completed, None)),
                "{finishes:?}"
            );
        }
    }

    #[test]
    fn given_stream_without_registered_first_then_nothing_served() {
        let setup = setup();

        let outcome = serve_stream(
            vec![RunnerEvent::Job(dispatched("train"))].into_iter(),
            &setup.executor,
        );
        setup.executor.wait_idle();

        assert!(matches!(outcome, StreamOutcome::NeverRegistered));
        assert!(setup.sink.finishes.lock().unwrap().is_empty());
    }
}
