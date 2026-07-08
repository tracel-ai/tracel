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

/// A structured log record, optionally scoped to an activity.
#[derive(Debug, Clone)]
pub struct LogRecord {
    pub level: LogLevel,
    pub message: String,
    pub attributes: Map<String, Value>,
    pub activity_id: Option<ActivityId>,
}

impl LogRecord {
    /// An unscoped informational record with no attributes.
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            level: LogLevel::Info,
            message: message.into(),
            attributes: Map::new(),
            activity_id: None,
        }
    }

    /// Render the record as a single line.
    pub fn render(&self) -> String {
        if self.attributes.is_empty() {
            self.message.clone()
        } else {
            let attributes = self
                .attributes
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {}", self.message, attributes)
        }
    }
}
