//! Cancelling the running job when the process is asked to stop.

use tracel_experiment::CancelToken;

use super::error::CANCELLED;

/// Cancels `cancel_token` on the first termination signal: SIGTERM, SIGINT or SIGHUP, or Ctrl-C,
/// Ctrl-Break or closing the console on Windows. A second one ends the process at once, with the
/// exit code of a cancelled job.
///
/// The handler is process-wide. When the program has installed its own, the job runs without
/// this one.
pub fn cancel_on_termination(cancel_token: CancelToken) {
    let installed = ctrlc::set_handler(move || {
        if cancel_token.is_cancelled() {
            std::process::exit(i32::from(CANCELLED));
        }
        eprintln!("Cancelling the job; signal again to stop at once.");
        cancel_token.cancel();
    });
    if let Err(error) = installed {
        eprintln!("warning: the job cannot be cancelled by a signal: {error}");
    }
}
