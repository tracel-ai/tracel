//! The job a thread runs: what an adapter gives a job, for one execution of it.
//!
//! Each execution of a job has a scope of its own, current on the thread that runs the job's
//! function, while it runs. Code in the job reads it through the functions below, or holds it,
//! through [`JobScope::current`], to read it on another thread. While a thread runs no job, such as
//! in a plain binary or a unit test, there is none, and the functions say so.

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
    use super::*;

    fn scope(num: &str) -> JobScope {
        new(Some(num.to_string()), serde_json::json!({"num": num}))
    }

    #[test]
    fn a_scope_is_current_on_its_thread_until_its_guard_drops() {
        let entered = enter(scope("12"));
        let given = (job_running(), job_num(), job_input());
        drop(entered);

        assert_eq!(
            given,
            (
                true,
                Some("12".to_string()),
                Some(serde_json::json!({"num": "12"}))
            )
        );
        assert_eq!((job_running(), job_num(), job_input()), (false, None, None));
    }

    #[test]
    fn each_thread_sees_the_scope_of_the_job_it_runs() {
        let run = |num: &'static str| {
            let scope = scope(num);
            std::thread::spawn(move || {
                let _entered = enter(scope);
                (job_num(), job_input())
            })
        };

        let one = run("1");
        let two = run("2");

        let input = |num: &str| Some(serde_json::json!({ "num": num }));
        assert_eq!(one.join().unwrap(), (Some("1".to_string()), input("1")));
        assert_eq!(two.join().unwrap(), (Some("2".to_string()), input("2")));
        assert!(!job_running());
    }

    #[test]
    fn a_held_scope_reads_the_same_job_on_another_thread() {
        let _entered = enter(scope("12"));
        let held = JobScope::current().unwrap();

        let seen = std::thread::spawn(move || {
            (
                job_running(),
                held.num().map(str::to_string),
                held.input().clone(),
            )
        })
        .join()
        .unwrap();

        assert_eq!(
            seen,
            (
                false,
                Some("12".to_string()),
                serde_json::json!({"num": "12"})
            )
        );
    }
}
