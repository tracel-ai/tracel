//! Reusable utilities for cancellable operations
//!
//! This module provides abstractions for common cancellation patterns to avoid
//! repetitive poll-and-cancel loops throughout the codebase.

use std::io::Read;
use std::process::{Child, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Cancellation token for controlling execution
#[derive(Debug, Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new cancellation token
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Cancel the execution.
    ///
    /// *Cancel Marcel.*
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Check if cancellation has been requested
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of a cancellable operation
#[derive(Debug)]
pub enum CancellableResult<T> {
    /// Operation completed successfully
    Completed(T),
    /// Operation was cancelled
    Cancelled,
}

impl<T> CancellableResult<T> {
    /// Check if the operation was cancelled
    pub fn is_cancelled(&self) -> bool {
        matches!(self, CancellableResult::Cancelled)
    }

    /// Convert to Result, mapping Cancelled to an error
    pub fn into_result(self, cancelled_error: impl Into<String>) -> Result<T, String> {
        match self {
            CancellableResult::Completed(value) => Ok(value),
            CancellableResult::Cancelled => Err(cancelled_error.into()),
        }
    }

    /// Convert to Result with a default cancelled error message
    pub fn into_result_default(self) -> Result<T, String> {
        self.into_result("Operation was cancelled")
    }
}

/// Utility for running cancellable child processes
pub struct CancellableProcess {
    child: Arc<Mutex<Option<Child>>>,
    cancellation_token: CancellationToken,
}

impl CancellableProcess {
    /// Create a new cancellable process wrapper
    pub fn new(child: Child, cancellation_token: CancellationToken) -> Self {
        Self {
            child: Arc::new(Mutex::new(Some(child))),
            cancellation_token,
        }
    }

    /// Wait for the process to complete or be cancelled
    ///
    /// This method captures both stdout and stderr output if available.
    ///
    /// This method polls the process every 100ms and returns either:
    /// - `CancellableResult::Completed(output)` if the process completes
    /// - `CancellableResult::Cancelled` if cancellation is requested
    pub fn wait_with_output(self) -> CancellableResult<std::process::Output> {
        let poll_interval = Duration::from_millis(100);

        let (stdout_tx, stdout_rx) = std::sync::mpsc::channel();
        let stdout = self.child.lock().unwrap().as_mut().unwrap().stdout.take();
        if let Some(mut out) = stdout {
            std::thread::spawn(move || {
                let mut buffer = Vec::new();
                let _ = out.read_to_end(&mut buffer);
                let _ = stdout_tx.send(buffer);
            });
        } else {
            drop(stdout_tx);
        }

        let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
        let stderr = self.child.lock().unwrap().as_mut().unwrap().stderr.take();
        if let Some(mut err) = stderr {
            std::thread::spawn(move || {
                let mut buffer = Vec::new();
                let _ = err.read_to_end(&mut buffer);
                let _ = stderr_tx.send(buffer);
            });
        } else {
            drop(stderr_tx);
        }

        let mut status = None;
        loop {
            if self.cancellation_token.is_cancelled() {
                self.kill_process();
                return CancellableResult::Cancelled;
            }

            let finished = if let Ok(mut child_opt) = self.child.lock() {
                if let Some(ref mut child) = child_opt.as_mut() {
                    match child.try_wait() {
                        Ok(Some(s)) => {
                            status = Some(s);
                            true
                        }
                        Ok(None) => false,
                        Err(_) => true,
                    }
                } else {
                    true
                }
            } else {
                false
            };

            if finished {
                break;
            }

            thread::sleep(poll_interval);
        }

        CancellableResult::Completed(Output {
            status: status.unwrap(),
            stdout: stdout_rx.recv().unwrap_or_default(),
            stderr: stderr_rx.recv().unwrap_or_default(),
        })
    }

    /// Wait for the process to complete or be cancelled (without capturing output)
    ///
    /// This method captures and discards both stdout and stderr output if available.
    ///
    /// This method polls the process every 100ms and returns either:
    /// - `CancellableResult::Completed(exit_status)` if the process completes
    /// - `CancellableResult::Cancelled` if cancellation is requested
    pub fn wait(self) -> CancellableResult<std::process::ExitStatus> {
        let poll_interval = Duration::from_millis(100);

        let stdout = self.child.lock().unwrap().as_mut().unwrap().stdout.take();
        if let Some(mut out) = stdout {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut out, &mut std::io::sink());
            });
        }

        let stderr = self.child.lock().unwrap().as_mut().unwrap().stderr.take();
        if let Some(mut err) = stderr {
            std::thread::spawn(move || {
                let _ = std::io::copy(&mut err, &mut std::io::sink());
            });
        }

        loop {
            if self.cancellation_token.is_cancelled() {
                self.kill_process();
                return CancellableResult::Cancelled;
            }

            if let Ok(mut child_opt) = self.child.lock() {
                if let Some(ref mut child) = child_opt.as_mut() {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            return CancellableResult::Completed(status);
                        }
                        Ok(None) => {}
                        Err(_) => {
                            return CancellableResult::Cancelled;
                        }
                    }
                }
            }

            thread::sleep(poll_interval);
        }
    }

    /// Force kill the process if it's still running
    fn kill_process(&self) {
        if let Ok(mut child_opt) = self.child.lock() {
            if let Some(ref mut child) = child_opt.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Utility for cancellable polling operations
pub struct CancellablePoller {
    cancellation_token: CancellationToken,
    poll_interval: Duration,
}

impl CancellablePoller {
    /// Create a new cancellable poller with default 100ms interval
    pub fn new(cancellation_token: CancellationToken) -> Self {
        Self {
            cancellation_token,
            poll_interval: Duration::from_millis(100),
        }
    }

    /// Create a poller with custom poll interval
    pub fn with_interval(cancellation_token: CancellationToken, interval: Duration) -> Self {
        Self {
            cancellation_token,
            poll_interval: interval,
        }
    }

    /// Poll a closure until it returns Some(value) or cancellation is requested
    ///
    /// The closure should return:
    /// - `Some(value)` when the operation is complete
    /// - `None` when the operation should continue polling
    pub fn poll_until<T, F>(&self, mut check_fn: F) -> CancellableResult<T>
    where
        F: FnMut() -> Option<T>,
    {
        loop {
            if self.cancellation_token.is_cancelled() {
                return CancellableResult::Cancelled;
            }

            if let Some(result) = check_fn() {
                return CancellableResult::Completed(result);
            }

            thread::sleep(self.poll_interval);
        }
    }

    /// Run a potentially long operation with periodic cancellation checks
    ///
    /// This is useful for operations that can be broken into chunks where
    /// cancellation can be checked between chunks.
    pub fn run_with_checks<T, E, F>(&self, operation: F) -> CancellableResult<Result<T, E>>
    where
        F: FnOnce(&CancellationToken) -> Result<T, E>,
    {
        if self.cancellation_token.is_cancelled() {
            return CancellableResult::Cancelled;
        }

        let result = operation(&self.cancellation_token);

        if self.cancellation_token.is_cancelled() {
            CancellableResult::Cancelled
        } else {
            CancellableResult::Completed(result)
        }
    }
}

/// Quick check if cancellation is requested, returning early with anyhow::Error
///
/// This macro can be used for functions that return anyhow::Result.
///
/// # Example
/// ```rust
/// use burn_central_workspace::execution::cancellable::CancellationToken;
/// use burn_central_workspace::execution::cancellable::check_cancelled_anyhow;
///
/// fn my_operation(token: &CancellationToken) -> anyhow::Result<()> {
///     check_cancelled_anyhow!(token, "My operation was cancelled");
///
///     // Continue with operation...
///     Ok(())
/// }
/// ```
#[macro_export]
macro_rules! check_cancelled_anyhow {
    ($token:expr) => {
        if $token.is_cancelled() {
            return Err(anyhow::anyhow!("Operation was cancelled"));
        }
    };
    ($token:expr, $msg:expr) => {
        if $token.is_cancelled() {
            return Err(anyhow::anyhow!($msg));
        }
    };
}

pub use check_cancelled_anyhow;

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_cancellable_result_conversions() {
        let completed: CancellableResult<i32> = CancellableResult::Completed(42);
        assert!(!completed.is_cancelled());
        assert_eq!(completed.into_result_default().unwrap(), 42);

        let cancelled: CancellableResult<i32> = CancellableResult::Cancelled;
        assert!(cancelled.is_cancelled());
        assert!(cancelled.into_result_default().is_err());
    }

    #[test]
    fn test_cancellable_process_immediate_cancellation() {
        let token = CancellationToken::new();
        token.cancel(); // Cancel immediately

        // Create a long-running process (sleep)
        let child = Command::new("sleep")
            .arg("10")
            .spawn()
            .expect("Failed to spawn sleep command");

        let cancellable = CancellableProcess::new(child, token);
        let result = cancellable.wait();

        assert!(result.is_cancelled());
    }

    #[test]
    fn test_cancellable_poller_immediate_cancellation() {
        let token = CancellationToken::new();
        token.cancel(); // Cancel immediately

        let poller = CancellablePoller::new(token);
        let result = poller.poll_until(|| Some("result"));

        assert!(result.is_cancelled());
    }

    #[test]
    fn test_cancellable_poller_completion() {
        let token = CancellationToken::new();
        let poller = CancellablePoller::new(token);

        let mut counter = 0;
        let result = poller.poll_until(|| {
            counter += 1;
            if counter >= 3 {
                Some("completed")
            } else {
                None
            }
        });

        match result {
            CancellableResult::Completed(value) => assert_eq!(value, "completed"),
            CancellableResult::Cancelled => panic!("Should not be cancelled"),
        }
    }

    #[test]
    fn test_cancellable_poller_with_background_cancellation() {
        let token = CancellationToken::new();
        let poller = CancellablePoller::new(token.clone());

        // Cancel after a short delay
        let cancel_token = token.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancel_token.cancel();
        });

        // This should be cancelled before completing
        let result = poller.poll_until(|| -> Option<String> {
            thread::sleep(Duration::from_millis(20));
            None // Never complete
        });

        assert!(result.is_cancelled());
    }

    #[test]
    fn test_check_cancelled_anyhow_macro() {
        let token = CancellationToken::new();

        // Should not return early when not cancelled
        let result = || -> anyhow::Result<i32> {
            check_cancelled_anyhow!(&token);
            Ok(42)
        }();
        assert_eq!(result.unwrap(), 42);

        // Should return early when cancelled
        token.cancel();
        let result = || -> anyhow::Result<i32> {
            check_cancelled_anyhow!(&token, "Custom cancellation message");
            Ok(42)
        }();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Custom cancellation message")
        );
    }

    mod token {
        use super::*;

        use std::thread;
        use std::time::Duration;

        #[test]
        fn test_cancellation_token_basic() {
            let token = CancellationToken::new();
            assert!(!token.is_cancelled());

            token.cancel();
            assert!(token.is_cancelled());
        }

        #[test]
        fn test_cancellation_token_clone() {
            let token = CancellationToken::new();
            let token_clone = token.clone();

            assert!(!token.is_cancelled());
            assert!(!token_clone.is_cancelled());

            token.cancel();
            assert!(token.is_cancelled());
            assert!(token_clone.is_cancelled());
        }

        #[test]
        fn test_cancellation_token_thread_safety() {
            let token = CancellationToken::new();
            let token_clone = token.clone();

            let handle = thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                token_clone.cancel();
            });

            // Wait for background thread to cancel
            handle.join().unwrap();

            // Should be cancelled now
            assert!(token.is_cancelled());
        }
    }
}
