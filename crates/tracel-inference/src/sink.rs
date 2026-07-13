//! Telemetry sink port. Implementations must be cheap to clone and non-blocking, since they are
//! called from the inference worker thread.

use serde_json::Value;

/// Current wall-clock time as Unix epoch milliseconds.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Kind of a metric, used to describe a metric name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricKind {
    Gauge,
    Counter,
    Distribution,
}

/// Severity of a log sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone)]
pub enum MetricData {
    /// A point-in-time value.
    Gauge { value: f64 },
    /// A monotonic increment (a delta, not an absolute total).
    Counter { value: u64 },
    /// A raw observation; quantiles are computed server-side at query time.
    Distribution { value: f64 },
}

/// A single metric sample scoped by its `metadata` attributes.
#[derive(Debug, Clone)]
pub struct MetricSample {
    pub name: String,
    pub timestamp_ms: i64,
    pub metadata: Value,
    pub data: MetricData,
}

/// A descriptor attached to a metric name (unit, human description).
#[derive(Debug, Clone)]
pub struct MetricDescriptor {
    pub name: String,
    pub kind: MetricKind,
    pub unit: Option<String>,
    pub description: Option<String>,
}

/// A single log sample scoped by its `metadata` attributes.
#[derive(Debug, Clone)]
pub struct LogSample {
    pub timestamp_ms: i64,
    pub level: LogLevel,
    pub message: String,
    pub metadata: Value,
}

/// Destination for a session's telemetry. Must not block.
pub trait InferenceSink: Send + Sync + 'static {
    /// Record a metric sample.
    fn record_metric(&self, sample: MetricSample);
    /// Record a log sample.
    fn record_log(&self, sample: LogSample);
    /// Declare a metric descriptor. Optional; downstream deduplicates by name.
    fn record_descriptor(&self, _descriptor: MetricDescriptor) {}
}

/// A sink that discards everything. Used by offline/local providers.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopSink;

impl InferenceSink for NoopSink {
    fn record_metric(&self, _sample: MetricSample) {}
    fn record_log(&self, _sample: LogSample) {}
}
