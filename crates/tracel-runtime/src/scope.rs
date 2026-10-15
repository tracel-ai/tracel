//! The job a thread runs: what an adapter gives a job, for one execution of it.
//!
//! Each execution of a job has a scope of its own, current on the thread that runs the job's
//! function, while it runs. Code in the job reads it through the functions below, or holds it,
//! through [`JobScope::current`], to read it on another thread. Outside a job there is none, and the
//! functions say so.

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::Value;

thread_local! {
    /// The scope of the job this thread runs, while it runs.
    static CURRENT: RefCell<Option<JobScope>> = const { RefCell::new(None) };
}

/// The job one execution runs: the number or id its launcher gave it, and its input. Clones share
/// the execution.
#[derive(Clone)]
pub struct JobScope {
    inner: Arc<Scope>,
}

struct Scope {
    num: Option<String>,
    input: Value,
}

impl JobScope {
    /// The scope of the job the current thread runs, while it runs one.
    pub fn current() -> Option<Self> {
        CURRENT.with(|current| current.borrow().clone())
    }

    /// The number or id the job's launcher gave it: the job's number for a process `tracel run`
    /// launched, the station's id for the job for a Station runner.
    pub fn num(&self) -> Option<&str> {
        self.inner.num.as_deref()
    }

    /// The input the job runs with, as JSON: the input its launcher gave merged onto its default,
    /// which its function received decoded; `Value::Null` when it was given none and has no
    /// default.
    pub fn input(&self) -> &Value {
        &self.inner.input
    }
}

/// The scope of a job numbered `num` by its launcher, running with `input`.
pub fn new(num: Option<String>, input: Value) -> JobScope {
    JobScope {
        inner: Arc::new(Scope { num, input }),
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

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::thread;

    use super::*;

    fn scope(num: &str) -> JobScope {
        new(Some(num.to_string()), serde_json::json!({"num": num}))
    }

    #[test]
    fn the_entered_scope_gives_the_number_and_input_of_its_job() {
        let job = scope("12");
        let _entered = enter(job.clone());

        assert_eq!(
            (job_num().as_deref(), job_input().as_ref()),
            (job.num(), Some(job.input()))
        );
    }

    #[test]
    fn once_its_guard_drops_the_thread_runs_no_job() {
        drop(enter(scope("12")));

        assert_eq!((job_running(), job_num(), job_input()), (false, None, None));
    }

    #[test]
    fn a_nested_scope_gives_way_to_the_one_before_when_its_guard_drops() {
        let outer = scope("1");
        let _outer = enter(outer.clone());

        drop(enter(scope("2")));

        assert_eq!(job_num().as_deref(), outer.num());
    }

    #[test]
    fn jobs_running_at_once_each_see_their_own_scope() {
        let both_entered = Arc::new(Barrier::new(2));
        let run = |job: JobScope| {
            let both_entered = both_entered.clone();
            thread::spawn(move || {
                let _entered = enter(job);
                both_entered.wait();
                job_num()
            })
        };
        let jobs = [scope("1"), scope("2")];

        let seen = jobs.clone().map(run).map(|running| running.join().unwrap());

        assert_eq!(seen, jobs.map(|job| job.num().map(str::to_string)));
    }

    #[test]
    fn a_held_scope_reads_the_same_job_on_another_thread() {
        let job = scope("12");
        let _entered = enter(job.clone());
        let held = JobScope::current().unwrap();

        let read = thread::spawn(move || (held.num().map(str::to_string), held.input().clone()))
            .join()
            .unwrap();

        assert_eq!(read, (job.num().map(str::to_string), job.input().clone()));
    }
}
