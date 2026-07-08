use std::fmt;
use std::sync::Arc;

use crate::observer::InferenceWriterObserver;

/// Opaque identifier for a single inference request/session.
///
/// The identifier format is backend-specific and stable only for the backend that created it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InferenceId(String);

impl InferenceId {
    /// Create an identifier from a backend-specific string value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the backend-specific identifier value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for InferenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for InferenceId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for InferenceId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

/// A per-request inference session.
///
/// A session is created by an [`InferenceProvider`](crate::InferenceProvider) for each inference
/// request and scopes that request's telemetry. It is intentionally minimal for now: it carries an
/// id and an [`InferenceWriterObserver`] that the job attaches to the request's output writer, so
/// the backend observes the request lifecycle (outputs, errors, duration) as it happens. More
/// session-scoped capabilities (structured events, artifacts) can be layered on later.
#[derive(Clone)]
pub struct InferenceSession {
    id: InferenceId,
    observer: Arc<dyn InferenceWriterObserver>,
}

impl InferenceSession {
    /// Create a session from an id and the telemetry observer that watches its request.
    pub fn new(id: impl Into<InferenceId>, observer: Arc<dyn InferenceWriterObserver>) -> Self {
        Self {
            id: id.into(),
            observer,
        }
    }

    /// Borrow the session identifier.
    pub fn id(&self) -> &InferenceId {
        &self.id
    }

    /// The observer to attach to the request's output writer.
    pub fn observer(&self) -> Arc<dyn InferenceWriterObserver> {
        self.observer.clone()
    }
}
