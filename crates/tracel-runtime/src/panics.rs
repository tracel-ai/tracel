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
    use std::io;

    use rstest::rstest;

    use super::*;

    const MESSAGE: &str = "the kernel exploded";

    #[test]
    fn a_panic_fails_the_run() {
        assert!(catch_panic(|| panic!("{MESSAGE}")).is_err());
    }

    #[test]
    fn an_error_the_run_returns_is_its_failure() {
        let failure = catch_panic(|| Err(io::Error::other(MESSAGE).into())).unwrap_err();

        assert!(failure.is::<io::Error>());
    }

    #[rstest]
    #[case::str(Box::new(MESSAGE))]
    #[case::string(Box::new(MESSAGE.to_string()))]
    fn the_panic_message_is_the_text_the_panic_carries(#[case] payload: Box<dyn Any + Send>) {
        assert_eq!(panic_message(payload.as_ref()), MESSAGE);
    }
}
