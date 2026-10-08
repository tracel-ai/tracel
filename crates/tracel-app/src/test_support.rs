use std::sync::{Mutex, MutexGuard, PoisonError};

/// Keeps other tests from running a job until the returned guard drops: the job a process runs is
/// the process's.
pub fn one_job_at_a_time() -> MutexGuard<'static, ()> {
    static ONE_JOB: Mutex<()> = Mutex::new(());
    ONE_JOB.lock().unwrap_or_else(PoisonError::into_inner)
}
