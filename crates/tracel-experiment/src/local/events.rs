use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::session::Event;
use crate::{ActivityEvent, ActivitySpec, ActivityStatus, MetricSpec, MetricValue};

/// The file a run appends its events to, one JSON object per line.
pub const EVENTS_FILE: &str = "events.jsonl";

/// One line of `events.jsonl`, with the fields the console's experiment API gives the event.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Line {
    Attribute {
        key: String,
        value: Value,
    },
    Arguments {
        value: Value,
    },
    Config {
        name: String,
        value: Value,
    },
    Log {
        timestamp: String,
        level: &'static str,
        message: String,
        metadata: Map<String, Value>,
        activity: Option<u64>,
    },
    Metric {
        name: String,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: String,
        activity: Option<u64>,
    },
    MetricDefinition {
        name: String,
        description: Option<String>,
        unit: Option<String>,
        higher_is_better: bool,
    },
    EpochSummary {
        name: String,
        epoch: usize,
        group: String,
        value: f64,
        activity: Option<u64>,
    },
    Summary {
        name: String,
        value: f64,
        activity: Option<u64>,
    },
    InputUsed {
        experiment: String,
        name: String,
        artifact_id: String,
    },
    ActivityStarted {
        activity: ActivitySpec,
    },
    ActivityUpdated {
        id: u64,
        current: u64,
    },
    ActivityMessage {
        id: u64,
        message: String,
    },
    ActivityFinished {
        id: u64,
        status: ActivityStatus,
        message: Option<String>,
    },
}

/// The lines `event` is recorded as: one, or one per value of a metric event.
pub fn lines(event: Event) -> Vec<Line> {
    match event {
        Event::Args(value) => vec![Line::Arguments { value }],
        Event::Config { name, value } => vec![Line::Config { name, value }],
        Event::Log { record, activity } => vec![Line::Log {
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            level: record.level.as_str(),
            message: record.message,
            metadata: record.attributes,
            activity: activity.map(|id| id.as_u64()),
        }],
        Event::Metrics {
            epoch,
            split,
            iteration,
            items,
            activity,
        } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::Metric {
                name,
                epoch,
                iteration,
                value,
                group: split.clone(),
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::MetricDefinition(MetricSpec {
            name,
            description,
            unit,
            higher_is_better,
        }) => vec![Line::MetricDefinition {
            name,
            description,
            unit,
            higher_is_better,
        }],
        Event::EpochSummary {
            epoch,
            split,
            items,
            activity,
        } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::EpochSummary {
                name,
                epoch,
                group: split.clone(),
                value,
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::Summary { items, activity } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::Summary {
                name,
                value,
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::ArtifactUsed {
            experiment_id,
            reference,
        } => vec![Line::InputUsed {
            experiment: experiment_id.to_string(),
            name: reference.name,
            artifact_id: reference.id,
        }],
        Event::Activity(event) => vec![match event {
            ActivityEvent::Started { activity } => Line::ActivityStarted { activity },
            ActivityEvent::Updated { id, current } => Line::ActivityUpdated {
                id: id.as_u64(),
                current,
            },
            ActivityEvent::Message { id, message } => Line::ActivityMessage {
                id: id.as_u64(),
                message,
            },
            ActivityEvent::Finished {
                id,
                status,
                message,
            } => Line::ActivityFinished {
                id: id.as_u64(),
                status,
                message,
            },
        }],
    }
}

/// `lines` as JSON Lines: each one JSON object followed by a newline.
pub fn to_jsonl(lines: impl IntoIterator<Item = Line>) -> serde_json::Result<String> {
    let mut jsonl = String::new();
    for line in lines {
        jsonl.push_str(&serde_json::to_string(&line)?);
        jsonl.push('\n');
    }
    Ok(jsonl)
}
