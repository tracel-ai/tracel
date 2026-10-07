//! A job that panics, as the failure it ends with.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::BoxError;

/// Runs `run`, which runs a job, and fails with the panic's message when it panics.
pub fn catch_panic(run: impl FnOnce() -> Result<(), BoxError>) -> Result<(), BoxError> {
    catch_unwind(AssertUnwindSafe(run)).unwrap_or_else(|panic| {
        Err(format!("the job panicked: {}", panic_message(panic.as_ref())).into())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_is_the_failure_the_job_ends_with() {
        let failure = |run: fn() -> Result<(), BoxError>| catch_panic(run).unwrap_err().to_string();

        assert_eq!(
            failure(|| panic!("kernel exploded")),
            "the job panicked: kernel exploded"
        );
        assert_eq!(
            failure(|| panic!("{} exploded", "kernel")),
            "the job panicked: kernel exploded"
        );
        assert_eq!(failure(|| Err("boom".into())), "boom");
        assert!(catch_panic(|| Ok(())).is_ok());
    }
}
