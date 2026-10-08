//! The runner loop: SSE reader, an executor running each dispatched job on a thread of its own,
//! reconnect with backoff.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tracel_job::ReportedExperiment;
use uuid::Uuid;

use crate::Jobs;
use crate::jobs::station::error::RunnerError;
use crate::jobs::station::infrastructure::protocol::{
    DispatchedJob, FinishJob, FinishStatus, RegisterRunner, RunnerEvent,
};
use crate::jobs::station::infrastructure::{ClientError, StationRunnerClient};
use crate::jobs::{Launch, PrepareError};
use crate::stop::Stop;

/// Reports job outcomes back to the station. Split out so the loop is testable with a fake.
pub trait FinishSink: Send + Sync + 'static {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob);
}

impl FinishSink for StationRunnerClient {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob) {
        // A rejected finish is expected after a disconnect: the station has already failed the
        // job and this session no longer owns it.
        if let Err(e) = StationRunnerClient::finish_job(self, job_id, finish) {
            eprintln!("warning: failed to report the outcome of job {job_id}: {e}");
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

fn execute(jobs: &Jobs, job: &DispatchedJob) -> (FinishStatus, Option<String>) {
    let ready = match jobs.prepare(&job.job_name, job.input.clone()) {
        Ok(ready) => ready,
        // Unreachable under the station's strict policy; defend anyway.
        Err(PrepareError::UnknownJob) => {
            return (
                FinishStatus::Failed,
                Some(format!("unknown job '{}'", job.job_name)),
            );
        }
        Err(PrepareError::InvalidInput(error)) => {
            return (
                FinishStatus::Failed,
                Some(format!("invalid input: {error}")),
            );
        }
    };
    eprintln!("Running job {} ({})", job.job_name, job.id);
    let launch = Launch {
        stop: Stop::default(),
        num: Some(job.id.to_string()),
        experiments: Arc::new(log_experiment(job.id)),
    };
    match crate::jobs::run(ready, launch) {
        Ok(()) => (FinishStatus::Completed, None),
        Err(error) => (FinishStatus::Failed, Some(error.to_string())),
    }
}

/// Logs each experiment the job `job_id` reports, as the station keeps no record of them.
fn log_experiment(job_id: Uuid) -> impl Fn(ReportedExperiment) + Send + Sync {
    move |experiment| {
        let num = experiment
            .num
            .map_or_else(|| "?".to_string(), |num| num.to_string());
        let place = experiment
            .url
            .or_else(|| experiment.dir.map(|dir| dir.display().to_string()));
        match place {
            Some(place) => eprintln!("Job {job_id} started experiment {num}: {place}"),
            None => eprintln!("Job {job_id} started experiment {num}"),
        }
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
    mut events: impl Iterator<Item = Result<RunnerEvent, ClientError>>,
    executor: &Executor,
) -> StreamOutcome {
    let runner_id = match events.next() {
        Some(Ok(RunnerEvent::Registered { runner_id })) => runner_id,
        Some(Ok(event)) => {
            eprintln!("warning: expected a registered event first, got {event:?}");
            return StreamOutcome::NeverRegistered;
        }
        Some(Err(e)) => {
            eprintln!("warning: the event stream failed before registration: {e}");
            return StreamOutcome::NeverRegistered;
        }
        None => {
            eprintln!("warning: the event stream closed before registration");
            return StreamOutcome::NeverRegistered;
        }
    };
    eprintln!("Registered with the station as runner {runner_id}");

    for event in events {
        match event {
            Ok(RunnerEvent::Job(job)) => executor.dispatch(runner_id, job),
            Ok(RunnerEvent::Registered { .. }) => {
                eprintln!("warning: unexpected registered event mid-stream");
            }
            Err(e) => {
                eprintln!("warning: the event stream failed: {e}");
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
            Err(e) => eprintln!("warning: failed to connect to the station: {e}"),
        }
        let delay = backoff.next();
        eprintln!("Reconnecting to the station in {}s", delay.as_secs());
        std::thread::sleep(delay);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::{BoxError, job_num};

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
                seen.lock().unwrap().push((job_num(), *count == 2));
                Ok::<_, BoxError>(())
            }
        });
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::new(Arc::new(jobs), sink.clone());
        let (first, second) = (dispatched("wait"), dispatched("wait"));

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

        let mut seen = seen.lock().unwrap().clone();
        seen.sort();
        let mut expected = vec![
            (Some(first.id.to_string()), true),
            (Some(second.id.to_string()), true),
        ];
        expected.sort();
        assert_eq!(seen, expected);
        assert_eq!(sink.finishes.lock().unwrap().len(), 2);
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
