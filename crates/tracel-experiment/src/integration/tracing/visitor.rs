use std::num::NonZeroU64;

use tracing::field::{Field, Visit};

use crate::{ActivityId, ExperimentId};

/// Extracts `experiment_id` and `activity_id` from span fields.
#[derive(Debug, Default)]
struct SpanFieldVisitor {
    experiment_id: Option<ExperimentId>,
    activity_id: Option<ActivityId>,
}

impl SpanFieldVisitor {
    fn record_experiment_id(&mut self, field: &Field, value: String) {
        if field.name() == "experiment_id" {
            self.experiment_id = Some(ExperimentId::new(value));
        }
    }

    fn record_activity_id(&mut self, field: &Field, value: u64) {
        if field.name() == "activity_id" {
            self.activity_id = NonZeroU64::new(value).map(ActivityId::new);
        }
    }
}

impl Visit for SpanFieldVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_activity_id(field, value);
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if let Ok(value) = u64::try_from(value) {
            self.record_activity_id(field, value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_experiment_id(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_experiment_id(field, format!("{value:?}"));
    }
}

/// Routing and scoping identifiers stored in a span's extensions.
#[derive(Debug, Clone, Default)]
pub struct SpanFields {
    pub experiment_id: Option<ExperimentId>,
    pub activity_id: Option<ActivityId>,
}

impl SpanFields {
    pub fn merge(&mut self, other: Self) {
        if other.experiment_id.is_some() {
            self.experiment_id = other.experiment_id;
        }
        if other.activity_id.is_some() {
            self.activity_id = other.activity_id;
        }
    }

    pub fn from_attributes(attrs: &tracing::span::Attributes<'_>) -> Self {
        let mut visitor = SpanFieldVisitor::default();
        attrs.record(&mut visitor);
        Self {
            experiment_id: visitor.experiment_id,
            activity_id: visitor.activity_id,
        }
    }

    pub fn from_record(record: &tracing::span::Record<'_>) -> Self {
        let mut visitor = SpanFieldVisitor::default();
        record.record(&mut visitor);
        Self {
            experiment_id: visitor.experiment_id,
            activity_id: visitor.activity_id,
        }
    }
}
