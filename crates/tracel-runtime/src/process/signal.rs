//! Asking the running job to stop when the process gets a termination signal.

use tracel_job_proto::ExitCode;

use crate::Stop;

/// Requests `stop`, which asks the job to stop, on the first termination signal: SIGTERM, SIGINT
/// or SIGHUP, or Ctrl-C, Ctrl-Break or closing the console on Windows. A second one ends the
/// process at once, with the exit code of a job asked to stop.
///
/// The handler is process-wide. When the program has installed its own, the job runs without
/// this one.
pub fn stop_on_termination(stop: Stop) {
    let installed = ctrlc::set_handler(move || {
        if stop.is_requested() {
            std::process::exit(i32::from(ExitCode::Stopped.code()));
        }
        eprintln!("Asking the job to stop; signal again to stop it at once.");
        stop.request();
    });
    if let Err(error) = installed {
        eprintln!("warning: the job cannot be stopped by a signal: {error}");
    }
}
