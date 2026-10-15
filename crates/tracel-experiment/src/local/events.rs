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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use crate::local::LocalExperiments;
    use crate::local::test_support::read_lines;
    use crate::{ExperimentProvider, LogRecord, MetricSpec, MetricValue};

    #[test]
    fn a_run_records_each_event_as_a_line_of_json() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = LocalExperiments::new(dir.path())
            .create_experiment(
                "mnist".to_string(),
                HashMap::from([
                    ("tracel.job_num".to_string(), json!(12)),
                    ("kind".to_string(), json!("example")),
                ]),
            )
            .unwrap();

        experiment.log_args(&json!({"epochs": 2})).unwrap();
        experiment
            .log_config("optimizer", &json!({"lr": 0.01}))
            .unwrap();
        experiment.log_metric_definition(MetricSpec {
            name: "loss".to_string(),
            description: Some("training loss".to_string()),
            unit: None,
            higher_is_better: false,
        });
        let epoch = experiment.activity("epoch 1").meter(4, "batch").start();
        epoch.log(LogRecord::warn("slow batch").with("batch", 3));
        epoch.log_metric(
            1,
            "train",
            4,
            vec![
                MetricValue {
                    name: "loss".to_string(),
                    value: 0.5,
                },
                MetricValue {
                    name: "accuracy".to_string(),
                    value: 0.9,
                },
            ],
        );
        epoch.inc(1);
        epoch.finish_with_message("done");
        experiment.log_epoch_summary(
            1,
            "train",
            vec![MetricValue {
                name: "loss".to_string(),
                value: 0.4,
            }],
        );
        experiment.log_summary(vec![MetricValue {
            name: "accuracy".to_string(),
            value: 0.95,
        }]);
        experiment.finish().unwrap();

        let mut lines = read_lines(&dir.path().join("mnist/1/events.jsonl"));
        let timestamp = lines[6]["timestamp"].take();
        assert!(
            timestamp
                .as_str()
                .is_some_and(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok()),
            "{timestamp}"
        );
        assert_eq!(
            lines,
            [
                json!({"type": "attribute", "key": "kind", "value": "example"}),
                json!({"type": "attribute", "key": "tracel.job_num", "value": 12}),
                json!({"type": "arguments", "value": {"epochs": 2}}),
                json!({"type": "config", "name": "optimizer", "value": {"lr": 0.01}}),
                json!({
                    "type": "metric_definition",
                    "name": "loss",
                    "description": "training loss",
                    "unit": null,
                    "higher_is_better": false
                }),
                json!({
                    "type": "activity_started",
                    "activity": {
                        "id": 1,
                        "parent": null,
                        "name": "epoch 1",
                        "cancellable": false,
                        "meter": {"unit": "batch", "total": 4},
                        "attributes": {}
                    }
                }),
                json!({
                    "type": "log",
                    "timestamp": null,
                    "level": "warn",
                    "message": "slow batch",
                    "metadata": {"batch": 3},
                    "activity": 1
                }),
                json!({
                    "type": "metric",
                    "name": "loss",
                    "epoch": 1,
                    "iteration": 4,
                    "value": 0.5,
                    "group": "train",
                    "activity": 1
                }),
                json!({
                    "type": "metric",
                    "name": "accuracy",
                    "epoch": 1,
                    "iteration": 4,
                    "value": 0.9,
                    "group": "train",
                    "activity": 1
                }),
                json!({"type": "activity_updated", "id": 1, "current": 1}),
                json!({
                    "type": "activity_finished",
                    "id": 1,
                    "status": "Success",
                    "message": "done"
                }),
                json!({
                    "type": "epoch_summary",
                    "name": "loss",
                    "epoch": 1,
                    "group": "train",
                    "value": 0.4,
                    "activity": null
                }),
                json!({"type": "summary", "name": "accuracy", "value": 0.95, "activity": null}),
            ]
        );
    }
}
