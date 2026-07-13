use serde_json::{Map, Value};
use tracing::field::{Field, Visit};

use crate::ExperimentId;

/// Field names that route a record rather than describe it; they are not surfaced as attributes.
const EXPERIMENT_ID_FIELD: &str = "experiment_id";

/// Extracts routing identifiers (`experiment_id`, `activity_id`) and every other field as an
/// inheritable attribute from a span's fields.
#[derive(Debug, Default)]
struct SpanFieldVisitor {
    experiment_id: Option<ExperimentId>,
    attributes: Map<String, Value>,
}

impl SpanFieldVisitor {
    fn record(&mut self, field: &Field, value: Value) {
        match field.name() {
            EXPERIMENT_ID_FIELD => {
                if let Value::String(id) = value {
                    self.experiment_id = Some(ExperimentId::new(id));
                }
            }
            name => {
                self.attributes.insert(name.to_string(), value);
            }
        }
    }
}

impl Visit for SpanFieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, Value::String(value.to_string()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record(field, Value::from(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.record(field, Value::from(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.record(field, Value::from(value));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, Value::String(format!("{value:?}")));
    }
}

/// Routing identifiers and inherited attributes stored in a span's extensions.
#[derive(Debug, Clone, Default)]
pub struct SpanFields {
    pub experiment_id: Option<ExperimentId>,
    pub attributes: Map<String, Value>,
}

impl SpanFields {
    pub fn merge(&mut self, other: Self) {
        if other.experiment_id.is_some() {
            self.experiment_id = other.experiment_id;
        }
        self.attributes.extend(other.attributes);
    }

    pub fn from_attributes(attrs: &tracing::span::Attributes<'_>) -> Self {
        let mut visitor = SpanFieldVisitor::default();
        attrs.record(&mut visitor);
        Self::from(visitor)
    }

    pub fn from_record(record: &tracing::span::Record<'_>) -> Self {
        let mut visitor = SpanFieldVisitor::default();
        record.record(&mut visitor);
        Self::from(visitor)
    }
}

impl From<SpanFieldVisitor> for SpanFields {
    fn from(visitor: SpanFieldVisitor) -> Self {
        Self {
            experiment_id: visitor.experiment_id,
            attributes: visitor.attributes,
        }
    }
}
