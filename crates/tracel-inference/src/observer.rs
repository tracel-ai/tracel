use std::time::Duration;

/// Runtime-owned output statistics for a completed inference request.
#[derive(Debug, Clone, Copy)]
pub struct InferenceOutputStats {
    pub duration: Duration,
    pub outputs: usize,
    pub errors: usize,
    pub cancelled: bool,
}

/// Observer interface for output lifecycle events.
pub trait InferenceOutputObserver: Send + Sync + 'static {
    /// Called when an output is written.
    fn on_write(&self) {}
    /// Called when an error is signaled.
    fn on_error(&self) {}
    /// Called when the inference is cancelled.
    fn on_cancelled(&self) {}
    /// Called when the inference is finished, with the final output statistics.
    fn on_finish(&self, _stats: &InferenceOutputStats) {}
}
