//! Asking the running job to stop when the process gets a termination signal.

use tracel_experiment::CancelToken;

use super::error::STOPPED;

/// Cancels `cancel_token` on the first termination signal: SIGTERM, SIGINT or SIGHUP, or Ctrl-C,
/// Ctrl-Break or closing the console on Windows. A second one ends the process at once, with the
/// exit code of a job asked to stop.
///
/// The handler is process-wide. When the program has installed its own, the job runs without
/// this one.
pub fn cancel_on_termination(cancel_token: CancelToken) {
    let installed = ctrlc::set_handler(move || {
        if cancel_token.is_cancelled() {
            std::process::exit(i32::from(STOPPED));
        }
        eprintln!("Asking the job to stop; signal again to stop it at once.");
        cancel_token.cancel();
    });
    if let Err(error) = installed {
        eprintln!("warning: the job cannot be stopped by a signal: {error}");
    }
}
