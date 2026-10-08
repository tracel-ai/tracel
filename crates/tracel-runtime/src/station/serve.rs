//! The runner loop: SSE reader, an executor running each dispatched job on a thread of its own,
//! reconnect with backoff.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tracing::{info, warn};
use uuid::Uuid;

use crate::station::error::RunnerError;
use crate::station::infrastructure::protocol::{
    DispatchedJob, FinishJob, FinishStatus, RegisterRunner, RunnerEvent,
};
use crate::station::infrastructure::{ClientError, StationRunnerClient};
use crate::{Execution, Jobs, Stop};

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

/// The request that each job running stop, by its station id, paired with a condvar so
/// [`Executor::wait_idle`] can block until no job runs.
type InFlight = Arc<(Mutex<HashMap<Uuid, Stop>>, Condvar)>;

/// Runs each dispatched job on a thread of its own, in a scope of its own, so the event stream
/// keeps being read (for liveness and cancels) while user code runs, and jobs the station
/// dispatches at once run at once.
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
        let stop = Stop::new();
        self.in_flight
            .0
            .lock()
            .unwrap()
            .insert(job.id, stop.clone());
        let (jobs, sink, in_flight) =
            (self.jobs.clone(), self.sink.clone(), self.in_flight.clone());
        let job_id = job.id;
        let spawned = std::thread::Builder::new()
            .name(format!("job-{job_id}"))
            .spawn(move || {
                let (status, reason) = execute(&jobs, &job, stop);
                sink.finish_job(
                    job.id,
                    &FinishJob {
                        runner_id,
                        status,
                        reason,
                    },
                );
                finish_in_flight(&in_flight, job.id);
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
            finish_in_flight(&self.in_flight, job_id);
        }
    }

    /// Asks the job `job_id` to stop, when it runs here: requests its scope's stop, which runs
    /// what the job registered with [`on_stop`](crate::on_stop).
    pub fn cancel(&self, job_id: Uuid) {
        let stop = self.in_flight.0.lock().unwrap().get(&job_id).cloned();
        match stop {
            Some(stop) => {
                info!(%job_id, "asking the job to stop");
                stop.request();
            }
            None => {
                warn!(%job_id, "the station cancelled a job that does not run here")
            }
        }
    }

    /// Block until every in-flight job has finished and reported.
    pub fn wait_idle(&self) {
        let (running, finished) = &*self.in_flight;
        let mut running = running.lock().unwrap();
        while !running.is_empty() {
            running = finished.wait(running).unwrap();
        }
    }
}

fn finish_in_flight(in_flight: &InFlight, job_id: Uuid) {
    let (running, finished) = &**in_flight;
    running.lock().unwrap().remove(&job_id);
    finished.notify_all();
}

/// Runs `job`, asked to stop once `stop` is requested, and returns how it ended: cancelled once it
/// was asked to stop, whatever it returned, and otherwise completed or failed by what it returned;
/// with its error when it failed, cancelled or not.
fn execute(jobs: &Jobs, job: &DispatchedJob, stop: Stop) -> (FinishStatus, Option<String>) {
    let prepared = match jobs.prepare(&job.job_name, job.input.clone()) {
        Ok(prepared) => prepared,
        // An unknown job is unreachable under the station's strict policy; defend anyway.
        Err(error) => return (FinishStatus::Failed, Some(error.to_string())),
    };
    info!(job = %job.job_name, job_id = %job.id, "running the job");
    let execution = Execution::new()
        .with_num(job.id.to_string())
        .with_stop(stop);
    let outcome = prepared.run(execution);
    let reason = outcome.error().map(ToString::to_string);
    let status = if outcome.stop_requested() {
        FinishStatus::Cancelled
    } else if reason.is_some() {
        FinishStatus::Failed
    } else {
        FinishStatus::Completed
    };
    (status, reason)
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
    mut events: impl Iterator<Item = Result<RunnerEvent, ClientError>>,
    executor: &Executor,
) -> StreamOutcome {
    let runner_id = match events.next() {
        Some(Ok(RunnerEvent::Registered { runner_id })) => runner_id,
        Some(Ok(event)) => {
            warn!(?event, "expected a registered event first");
            return StreamOutcome::NeverRegistered;
        }
        Some(Err(e)) => {
            warn!(error = %e, "the event stream failed before registration");
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
            Ok(RunnerEvent::Job(job)) => executor.dispatch(runner_id, job),
            Ok(RunnerEvent::Cancel { job_id }) => executor.cancel(job_id),
            Ok(RunnerEvent::Registered { .. }) => {
                warn!("unexpected registered event mid-stream");
            }
            Err(e) => {
                warn!(error = %e, "the event stream failed");
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
    use crate::{BoxError, job_input, job_num, on_stop, stop_requested};

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

    fn events(
        runner_id: Uuid,
        rest: Vec<RunnerEvent>,
    ) -> impl Iterator<Item = Result<RunnerEvent, ClientError>> {
        std::iter::once(RunnerEvent::Registered { runner_id })
            .chain(rest)
            .map(Ok)
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

    /// A flag a job raises and another job, or the test, waits for, up to ten seconds.
    #[derive(Clone, Default)]
    struct Flag(Arc<(Mutex<bool>, Condvar)>);

    impl Flag {
        fn raise(&self) {
            *self.0.0.lock().unwrap() = true;
            self.0.1.notify_all();
        }

        /// Whether the flag was raised within ten seconds.
        fn wait(&self) -> bool {
            let (raised, changed) = &*self.0;
            let raised = changed
                .wait_timeout_while(raised.lock().unwrap(), Duration::from_secs(10), |raised| {
                    !*raised
                })
                .unwrap()
                .0;
            *raised
        }
    }

    /// A job that waits until it is asked to stop, then ends with `outcome`, and the flag it raises
    /// once asked to stop.
    fn stoppable(
        outcome: Result<(), &'static str>,
    ) -> (
        impl Fn(Value) -> Result<(), BoxError> + Send + Sync + 'static,
        Flag,
    ) {
        let stopped = Flag::default();
        let job = {
            let stopped = stopped.clone();
            move |_: Value| {
                let asked = Flag::default();
                on_stop({
                    let asked = asked.clone();
                    move || asked.raise()
                });
                if asked.wait() {
                    stopped.raise();
                }
                outcome.map_err(BoxError::from)
            }
        };
        (job, stopped)
    }

    /// Serves `events` after registration, waits for the jobs to end, and returns the finishes.
    fn serve_events(jobs: Jobs, rest: Vec<RunnerEvent>) -> Vec<RecordedFinish> {
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::new(Arc::new(jobs), sink.clone());
        serve_stream(events(Uuid::new_v4(), rest), &executor);
        executor.wait_idle();
        sink.finishes.lock().unwrap().clone()
    }

    #[test]
    fn a_cancelled_job_is_asked_to_stop_and_reported_cancelled() {
        let (job, stopped) = stoppable(Ok(()));
        let cancelled = dispatched("stoppable");

        let finishes = serve_events(
            Jobs::new().job("stoppable", job),
            vec![
                RunnerEvent::Job(cancelled.clone()),
                RunnerEvent::Cancel {
                    job_id: cancelled.id,
                },
            ],
        );

        assert!(stopped.wait());
        assert_eq!(finishes.len(), 1);
        assert_eq!(
            (finishes[0].0, finishes[0].2, finishes[0].3.clone()),
            (cancelled.id, FinishStatus::Cancelled, None)
        );
    }

    #[test]
    fn a_job_that_fails_once_cancelled_is_reported_cancelled_with_its_error() {
        let (job, _) = stoppable(Err("interrupted"));
        let cancelled = dispatched("stoppable");

        let finishes = serve_events(
            Jobs::new().job("stoppable", job),
            vec![
                RunnerEvent::Job(cancelled.clone()),
                RunnerEvent::Cancel {
                    job_id: cancelled.id,
                },
            ],
        );

        assert_eq!(finishes[0].2, FinishStatus::Cancelled);
        assert_eq!(finishes[0].3.as_deref(), Some("interrupted"));
    }

    #[test]
    fn a_cancel_stops_only_the_job_it_names() {
        let (stoppable, stopped) = stoppable(Ok(()));
        let seen = Arc::new(Mutex::new(None));
        let waiting = {
            let (stopped, seen) = (stopped.clone(), seen.clone());
            move |_: Value| {
                let other_stopped = stopped.wait();
                *seen.lock().unwrap() = Some((other_stopped, stop_requested(), job_num()));
                Ok::<_, BoxError>(())
            }
        };
        let (cancelled, other) = (dispatched("stoppable"), dispatched("waiting"));

        let finishes = serve_events(
            Jobs::new()
                .job("stoppable", stoppable)
                .job("waiting", waiting),
            vec![
                RunnerEvent::Job(cancelled.clone()),
                RunnerEvent::Job(other.clone()),
                RunnerEvent::Cancel {
                    job_id: cancelled.id,
                },
            ],
        );

        assert_eq!(
            *seen.lock().unwrap(),
            Some((true, false, Some(other.id.to_string())))
        );
        let status = |id| finishes.iter().find(|finish| finish.0 == id).unwrap().2;
        assert_eq!(status(cancelled.id), FinishStatus::Cancelled);
        assert_eq!(status(other.id), FinishStatus::Completed);
    }

    #[test]
    fn a_cancel_for_a_job_that_does_not_run_here_changes_nothing() {
        let job = dispatched("train");

        let finishes = serve_events(
            jobs(Arc::default()),
            vec![
                RunnerEvent::Cancel {
                    job_id: Uuid::new_v4(),
                },
                RunnerEvent::Job(job.clone()),
            ],
        );

        assert_eq!(finishes.len(), 1);
        assert_eq!(finishes[0].2, FinishStatus::Completed);
    }

    #[test]
    fn given_stream_without_registered_first_then_nothing_served() {
        let setup = setup();

        let outcome = serve_stream(
            vec![Ok(RunnerEvent::Job(dispatched("train")))].into_iter(),
            &setup.executor,
        );
        setup.executor.wait_idle();

        assert!(matches!(outcome, StreamOutcome::NeverRegistered));
        assert!(setup.sink.finishes.lock().unwrap().is_empty());
    }
}
