//! The runner loop: SSE reader, an executor running each dispatched job on a thread of its own,
//! reconnect with backoff.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tracing::{info, warn};
use uuid::Uuid;

use crate::panics::catch_panic;
use crate::station::error::RunnerError;
use crate::station::infrastructure::StationRunnerClient;
use crate::station::infrastructure::protocol::{
    DispatchedJob, FinishJob, FinishStatus, RegisterRunner, RunnerEvent,
};
use crate::{Execution, Jobs};

/// Reports job outcomes back to the station.
pub trait FinishSink: Send + Sync + 'static {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob);
}

impl FinishSink for StationRunnerClient {
    fn finish_job(&self, job_id: Uuid, finish: &FinishJob) {
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
        let running = Running::start(&self.in_flight);
        let (jobs, sink) = (self.jobs.clone(), self.sink.clone());
        let job_id = job.id;
        let spawned = std::thread::Builder::new()
            .name(format!("job-{job_id}"))
            .spawn(move || {
                let _running = running;
                let (status, reason) = execute(&jobs, &job);
                sink.finish_job(
                    job.id,
                    &FinishJob {
                        runner_id,
                        status,
                        reason,
                    },
                );
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

/// A dispatched job, counted in flight until this drops, when its thread ends or unwinds.
struct Running(InFlight);

impl Running {
    fn start(in_flight: &InFlight) -> Self {
        *in_flight.0.lock().unwrap() += 1;
        Self(in_flight.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let (count, finished) = &*self.0;
        *count.lock().unwrap() -= 1;
        finished.notify_all();
    }
}

/// Runs `job` and returns how it ended: completed or failed by what it returned, with its error
/// when it failed. A panic fails it, even one while its input decodes.
fn execute(jobs: &Jobs, job: &DispatchedJob) -> (FinishStatus, Option<String>) {
    let ran = catch_panic(|| {
        let prepared = jobs.prepare(&job.job_name, job.input.clone())?;
        info!(job = %job.job_name, job_id = %job.id, "running the job");
        prepared
            .run(Execution::new().with_num(job.id.to_string()))
            .into_result()
    });
    match ran {
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
    use rstest::{fixture, rstest};
    use serde::{Deserialize, Deserializer};
    use serde_json::{Value, json};

    use std::sync::{Barrier, OnceLock};

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

    struct Explosive;

    impl<'de> Deserialize<'de> for Explosive {
        fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
            panic!("the decoder exploded")
        }
    }

    const FAILURE: &str = "boom";

    fn jobs() -> Jobs {
        let both = Arc::new(Barrier::new(2));
        Jobs::new()
            .job("train", |_: Value| Ok::<_, BoxError>(()))
            .job("fail", |_: Value| Err::<(), BoxError>(FAILURE.into()))
            .job("typed", |_: u32| Ok::<_, BoxError>(()))
            .job("explosive", |_: Explosive| Ok::<_, BoxError>(()))
            .job("wait", move |_: Value| {
                both.wait();
                Ok::<_, BoxError>(())
            })
    }

    struct Setup {
        executor: Executor,
        sink: Arc<RecordingSink>,
        runner_id: Uuid,
    }

    fn serving(jobs: Jobs) -> Setup {
        let sink = Arc::new(RecordingSink::default());
        let executor = Executor::new(Arc::new(jobs), sink.clone());
        Setup {
            executor,
            sink,
            runner_id: Uuid::new_v4(),
        }
    }

    #[fixture]
    fn setup() -> Setup {
        serving(jobs())
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

    fn serve(setup: &Setup, jobs: Vec<DispatchedJob>) -> Vec<RecordedFinish> {
        let rest = jobs.into_iter().map(RunnerEvent::Job).collect();
        serve_stream(events(setup.runner_id, rest), &setup.executor);
        setup.executor.wait_idle();
        finishes(setup)
    }

    fn finishes(setup: &Setup) -> Vec<RecordedFinish> {
        let mut finishes = setup.sink.finishes.lock().unwrap().clone();
        finishes.sort_by_key(|(job_id, ..)| *job_id);
        finishes
    }

    fn statuses(finishes: &[RecordedFinish]) -> Vec<(Uuid, FinishStatus)> {
        finishes
            .iter()
            .map(|(job_id, _, status, _)| (*job_id, *status))
            .collect()
    }

    fn completed<'a>(
        jobs: impl IntoIterator<Item = &'a DispatchedJob>,
    ) -> Vec<(Uuid, FinishStatus)> {
        let mut completed: Vec<_> = jobs
            .into_iter()
            .map(|job| (job.id, FinishStatus::Completed))
            .collect();
        completed.sort_by_key(|(job_id, _)| *job_id);
        completed
    }

    #[rstest]
    #[case::completed("train", FinishStatus::Completed)]
    #[case::failed("fail", FinishStatus::Failed)]
    #[case::with_an_input_that_does_not_decode("typed", FinishStatus::Failed)]
    #[case::unknown("mystery", FinishStatus::Failed)]
    fn a_job_is_reported_as_it_ended(
        setup: Setup,
        #[case] name: &str,
        #[case] status: FinishStatus,
    ) {
        let job = dispatched(name);

        let finishes = serve(&setup, vec![job.clone()]);

        assert_eq!(statuses(&finishes), [(job.id, status)]);
    }

    #[rstest]
    fn a_failed_job_is_reported_with_its_error_as_the_reason(setup: Setup) {
        let job = dispatched("fail");

        let finishes = serve(&setup, vec![job.clone()]);

        assert_eq!(
            finishes,
            [(
                job.id,
                setup.runner_id,
                FinishStatus::Failed,
                Some(FAILURE.to_string())
            )]
        );
    }

    #[rstest]
    fn a_panic_while_the_input_decodes_fails_the_job_and_frees_the_runner(setup: Setup) {
        let job = dispatched("explosive");

        let finishes = serve(&setup, vec![job.clone()]);

        assert_eq!(statuses(&finishes), [(job.id, FinishStatus::Failed)]);
    }

    #[test]
    fn a_running_job_is_numbered_with_its_station_job_id() {
        let seen = Arc::new(OnceLock::new());
        let setup = serving(Jobs::new().job("train", {
            let seen = seen.clone();
            move |_: Value| {
                let _ = seen.set(job_num());
                Ok::<_, BoxError>(())
            }
        }));
        let job = dispatched("train");

        serve(&setup, vec![job.clone()]);

        assert_eq!(seen.get(), Some(&Some(job.id.to_string())));
    }

    #[rstest]
    fn jobs_dispatched_at_once_run_at_once(setup: Setup) {
        let waiting = [dispatched("wait"), dispatched("wait")];

        let finishes = serve(&setup, waiting.to_vec());

        assert_eq!(statuses(&finishes), completed(&waiting));
    }

    #[rstest]
    fn a_cancelled_job_runs_to_completion_and_the_stream_goes_on(setup: Setup) {
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

        assert_eq!(statuses(&finishes(&setup)), completed([&cancelled, &next]));
    }

    #[rstest]
    fn a_stream_that_does_not_open_with_its_registration_serves_nothing(setup: Setup) {
        let outcome = serve_stream(
            vec![RunnerEvent::Job(dispatched("train"))].into_iter(),
            &setup.executor,
        );
        setup.executor.wait_idle();

        assert!(matches!(outcome, StreamOutcome::NeverRegistered));
        assert!(finishes(&setup).is_empty());
    }
}
