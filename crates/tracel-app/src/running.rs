//! The job this process runs, while an adapter runs it.
//!
//! An adapter runs one job at a time, so what the job is given is the process's: code that runs in
//! the job, on any thread, reads it through the functions below. While no job runs, such as in a
//! plain binary or a unit test, there is none, and the functions say so.

use std::sync::{Arc, PoisonError, RwLock};

use tracel_job::ReportedExperiment;

use crate::stop::Stop;

/// Where the experiments a job starts are reported.
pub type ExperimentSink = Arc<dyn Fn(ReportedExperiment) + Send + Sync>;

/// The job this process runs, while it runs.
static RUNNING: RwLock<Option<Arc<RunningJob>>> = RwLock::new(None);

/// What the job this process runs is given.
pub struct RunningJob {
    stop: Stop,
    num: Option<String>,
    experiments: ExperimentSink,
}

impl RunningJob {
    /// The job asked to stop once `stop` is requested, numbered `num` by its launcher, the
    /// experiments it starts reported to `experiments`.
    pub fn new(stop: Stop, num: Option<String>, experiments: ExperimentSink) -> Self {
        Self {
            stop,
            num,
            experiments,
        }
    }
}

/// Makes `job` the job this process runs, until the returned guard drops.
pub fn enter(job: RunningJob) -> Entered {
    *RUNNING.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(job));
    Entered
}

/// The job entered with [`enter`] runs until this drops.
pub struct Entered;

impl Drop for Entered {
    fn drop(&mut self) {
        *RUNNING.write().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn running() -> Option<Arc<RunningJob>> {
    RUNNING
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Whether this process runs a job.
pub fn job_running() -> bool {
    running().is_some()
}

/// Whether the job this process runs was asked to stop, which the first termination signal does;
/// `false` while the process runs no job.
pub fn stop_requested() -> bool {
    running().is_some_and(|job| job.stop.is_requested())
}

/// Runs `action` once the job this process runs is asked to stop, on the thread that asks it, or
/// at once when it already was.
///
/// While the process runs no job, `action` never runs.
pub fn on_stop(action: impl FnOnce() + Send + 'static) {
    if let Some(job) = running() {
        job.stop.on_request(Box::new(action));
    }
}

/// Asks the job this process runs to stop, as a termination signal does.
#[cfg(test)]
pub fn request_stop() {
    if let Some(job) = running() {
        job.stop.request();
    }
}

/// The number the launcher gave the job this process runs, through `TRACEL_JOB_NUM`; `None` while
/// the process runs no job, or one no launcher numbered.
pub fn job_num() -> Option<String> {
    running().and_then(|job| job.num.clone())
}

/// Reports `experiment`, which code in the job this process runs started, as the job's adapter
/// reports experiments: the process adds it to the job's run report, and a Station runner logs
/// it.
///
/// Does nothing while the process runs no job.
pub fn report_experiment(experiment: ReportedExperiment) {
    if let Some(job) = running() {
        (job.experiments)(experiment);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::test_support::one_job_at_a_time;

    #[test]
    fn a_running_job_is_given_until_it_ends() {
        let _one = one_job_at_a_time();
        let stop = Stop::default();
        let stopped = Arc::new(AtomicUsize::new(0));
        let count_on_stop = || {
            let stopped = stopped.clone();
            on_stop(move || {
                stopped.fetch_add(1, Ordering::SeqCst);
            });
        };

        let entered = enter(RunningJob::new(
            stop.clone(),
            Some("12".to_string()),
            Arc::new(|_| {}),
        ));
        let given = (job_running(), stop_requested(), job_num());
        count_on_stop();
        stop.request();
        let requested = (stop_requested(), stopped.load(Ordering::SeqCst));
        count_on_stop();
        let late = stopped.load(Ordering::SeqCst);
        drop(entered);
        count_on_stop();

        assert_eq!(given, (true, false, Some("12".to_string())));
        assert_eq!(requested, (true, 1));
        assert_eq!(late, 2);
        assert_eq!(
            (job_running(), stop_requested(), job_num()),
            (false, false, None)
        );
        assert_eq!(stopped.load(Ordering::SeqCst), 2);
    }
}
