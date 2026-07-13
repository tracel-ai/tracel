//! Structured log records emitted by a run.

use serde_json::{Map, Value};

use crate::activity::ActivityId;

/// Severity level of a [`LogRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// The lowercase name of the level.
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

/// A structured log record: a severity, a message, and a bag of attributes.
///
/// Build one directly with a level constructor and attach attributes fluently:
///
/// ```ignore
/// use tracel_experiment::LogRecord;
///
/// let record = LogRecord::warn("slow step").with("elapsed_ms", 900).with("split", "train");
/// ```
///
/// Attributes carry the structured metadata surfaced by the log viewer. Records may also be scoped
/// to an activity, in which case the activity id is folded into the attributes on the wire.
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
    pub attributes: Map<String, Value>,
    pub activity_id: Option<ActivityId>,
}

impl LogRecord {
    /// A record at `level` with `message` and no attributes.
    pub fn new(level: LogLevel, message: impl Into<String>) -> Self {
        Self {
            level,
            message: message.into(),
            attributes: Map::new(),
            activity_id: None,
        }
    }

    /// A `trace` record with no attributes.
    pub fn trace(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Trace, message)
    }

    /// A `debug` record with no attributes.
    pub fn debug(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Debug, message)
    }

    /// An `info` record with no attributes.
    pub fn info(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, message)
    }

    /// A `warn` record with no attributes.
    pub fn warn(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, message)
    }

    /// An `error` record with no attributes.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, message)
    }

    /// Attach a single attribute, returning the record for chaining.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Attach several attributes, returning the record for chaining.
    #[must_use]
    pub fn with_attrs(mut self, attrs: impl IntoIterator<Item = (String, Value)>) -> Self {
        self.attributes.extend(attrs);
        self
    }

    /// Fill in scope attributes that the record does not already define.
    ///
    /// Existing attributes win, so call-site fields always take precedence over inherited scope.
    pub(crate) fn inherit_attrs(&mut self, scope: &Map<String, Value>) {
        for (key, value) in scope {
            self.attributes
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
}
