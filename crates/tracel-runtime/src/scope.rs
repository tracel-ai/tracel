//! The job a thread runs: what an adapter gives a job, for one execution of it.
//!
//! Each execution of a job has a scope of its own, current on the thread that runs the job's
//! function, while it runs. Code in the job reads it through the functions below, or holds it,
//! through [`JobScope::current`], to read it on another thread. While a thread runs no job, such as
//! in a plain binary or a unit test, there is none, and the functions say so.

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::Value;
use tracel_job_proto::ReportedExperiment;

use crate::stop::Stop;

thread_local! {
    /// The scope of the job this thread runs, while it runs.
    static CURRENT: RefCell<Option<JobScope>> = const { RefCell::new(None) };
}

/// Where the experiments a job starts are reported.
pub type ExperimentSink = Arc<dyn Fn(ReportedExperiment) + Send + Sync>;

/// The job one execution runs: the number or id its launcher gave it, its input, the request that
/// it stop, and where the experiments it starts are reported. Clones share the execution.
#[derive(Clone)]
pub struct JobScope {
    inner: Arc<Scope>,
}

struct Scope {
    num: Option<String>,
    input: Value,
    stop: Stop,
    experiments: ExperimentSink,
}

impl JobScope {
    /// The scope of the job the current thread runs, while it runs one.
    pub fn current() -> Option<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }

    /// The number or id the job's launcher gave it: `TRACEL_JOB_NUM` for a process, the station's
    /// id for the job for a Station runner.
    pub fn num(&self) -> Option<&str> {
        self.inner.num.as_deref()
    }

    /// The input the job runs with, as JSON: the input its launcher gave merged onto its default,
    /// which its function received decoded; `Value::Null` when it was given none and has no
    /// default.
    pub fn input(&self) -> &Value {
        &self.inner.input
    }

    /// Whether the job was asked to stop.
    pub fn stop_requested(&self) -> bool {
        self.inner.stop.is_requested()
    }

    /// Runs `action` once the job is asked to stop, on the thread that asks it, or at once when it
    /// already was.
    pub fn on_stop(&self, action: impl FnOnce() + Send + 'static) {
        self.inner.stop.on_request(Box::new(action));
    }

    /// Reports `experiment`, which the job started, as its adapter reports experiments: the
    /// process adds it to the job's run report, and a Station runner logs it.
    pub fn report_experiment(&self, experiment: ReportedExperiment) {
        (self.inner.experiments)(experiment);
    }
}

/// The scope of a job numbered `num` by its launcher, running with `input`, asked to stop once
/// `stop` is requested, the experiments it starts reported to `experiments`.
pub fn new(num: Option<String>, input: Value, stop: Stop, experiments: ExperimentSink) -> JobScope {
    JobScope {
        inner: Arc::new(Scope {
            num,
            input,
            stop,
            experiments,
        }),
    }
}

/// Makes `scope` current on this thread, until the returned guard drops.
pub fn enter(scope: JobScope) -> Entered {
    let previous = CURRENT.with(|current| current.replace(Some(scope)));
    Entered { previous }
}

/// The scope entered with [`enter`] is current until this drops, when the one before is again.
pub struct Entered {
    previous: Option<JobScope>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| current.replace(previous));
    }
}

/// Whether the current thread runs a job.
pub fn job_running() -> bool {
    JobScope::current().is_some()
}

/// Whether the job the current thread runs was asked to stop; `false` while it runs none.
pub fn stop_requested() -> bool {
    JobScope::current().is_some_and(|scope| scope.stop_requested())
}

/// Runs `action` once the job the current thread runs is asked to stop, on the thread that asks
/// it, or at once when it already was.
///
/// While the thread runs no job, `action` never runs.
pub fn on_stop(action: impl FnOnce() + Send + 'static) {
    if let Some(scope) = JobScope::current() {
        scope.on_stop(action);
    }
}

/// The number or id the launcher gave the job the current thread runs; `None` while it runs none,
/// or one no launcher numbered.
pub fn job_num() -> Option<String> {
    JobScope::current().and_then(|scope| scope.num().map(str::to_string))
}

/// The input the job the current thread runs was given, as JSON, as [`JobScope::input`] gives it;
/// `None` while the thread runs no job.
pub fn job_input() -> Option<Value> {
    JobScope::current().map(|scope| scope.input().clone())
}

/// Reports `experiment`, which code in the job the current thread runs started, as the job's
/// adapter reports experiments: the process adds it to the job's run report, and a Station runner
/// logs it.
///
/// Does nothing while the thread runs no job.
pub fn report_experiment(experiment: ReportedExperiment) {
    if let Some(scope) = JobScope::current() {
        scope.report_experiment(experiment);
    }
}

/// Asks the job the current thread runs to stop, as its adapter does.
#[cfg(test)]
pub fn request_stop() {
    if let Some(scope) = JobScope::current() {
        scope.inner.stop.request();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn scope(num: &str, stop: &Stop, reported: &Arc<Mutex<Vec<ReportedExperiment>>>) -> JobScope {
        let reported = reported.clone();
        new(
            Some(num.to_string()),
            serde_json::json!({"num": num}),
            stop.clone(),
            Arc::new(move |experiment| reported.lock().unwrap().push(experiment)),
        )
    }

    fn experiment(num: u64) -> ReportedExperiment {
        ReportedExperiment {
            num: Some(num),
            url: None,
            dir: None,
        }
    }

    #[test]
    fn a_scope_is_current_on_its_thread_until_its_guard_drops() {
        let stop = Stop::default();
        let reported = Arc::default();
        let stopped = Arc::new(AtomicUsize::new(0));
        let count_on_stop = || {
            let stopped = stopped.clone();
            on_stop(move || {
                stopped.fetch_add(1, Ordering::SeqCst);
            });
        };

        let entered = enter(scope("12", &stop, &reported));
        let given = (job_running(), stop_requested(), job_num());
        let input = job_input();
        report_experiment(experiment(1));
        count_on_stop();
        stop.request();
        let requested = (stop_requested(), stopped.load(Ordering::SeqCst));
        count_on_stop();
        let late = stopped.load(Ordering::SeqCst);
        drop(entered);
        count_on_stop();
        report_experiment(experiment(2));

        assert_eq!(given, (true, false, Some("12".to_string())));
        assert_eq!(input, Some(serde_json::json!({"num": "12"})));
        assert_eq!(requested, (true, 1));
        assert_eq!(late, 2);
        assert_eq!(
            (job_running(), stop_requested(), job_num(), job_input()),
            (false, false, None, None)
        );
        assert_eq!(stopped.load(Ordering::SeqCst), 2);
        assert_eq!(*reported.lock().unwrap(), [experiment(1)]);
    }

    #[test]
    fn each_thread_sees_the_scope_of_the_job_it_runs() {
        let (first, second) = (Stop::default(), Stop::default());
        let reported = Arc::new(Mutex::new(Vec::new()));
        let run = |num: &'static str, stop: &Stop| {
            let scope = scope(num, stop, &reported);
            std::thread::spawn(move || {
                let _entered = enter(scope);
                (job_num(), job_input(), stop_requested())
            })
        };

        first.request();
        let one = run("1", &first);
        let two = run("2", &second);

        let input = |num: &str| Some(serde_json::json!({ "num": num }));
        assert_eq!(
            one.join().unwrap(),
            (Some("1".to_string()), input("1"), true)
        );
        assert_eq!(
            two.join().unwrap(),
            (Some("2".to_string()), input("2"), false)
        );
        assert!(!job_running());
    }

    #[test]
    fn a_held_scope_reads_the_same_job_on_another_thread() {
        let stop = Stop::default();
        let reported = Arc::new(Mutex::new(Vec::new()));
        let _entered = enter(scope("12", &stop, &reported));
        let held = JobScope::current().unwrap();

        let seen = std::thread::spawn(move || {
            held.report_experiment(experiment(1));
            (job_running(), held.num().map(str::to_string))
        })
        .join()
        .unwrap();

        assert_eq!(seen, (false, Some("12".to_string())));
        assert_eq!(*reported.lock().unwrap(), [experiment(1)]);
    }
}
