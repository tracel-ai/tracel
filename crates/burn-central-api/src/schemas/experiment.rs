use std::collections::HashMap;

use serde::Serialize;

use crate::metrics::MetricLog;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputUsed {
    Artifact { artifact_id: String },
    Model { model_version_id: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ExperimentCompletion {
    Success,
    Fail { reason: String },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ExperimentMessage {
    MetricsLog {
        epoch: usize,
        split: String,
        iteration: usize,
        items: Vec<MetricLog>,
    },
    MetricDefinitionLog {
        name: String,
        description: Option<String>,
        unit: Option<String>,
        higher_is_better: bool,
    },
    EpochSummaryLog {
        epoch: usize,
        split: String,
        best_metric_values: HashMap<String, f64>,
    },
    Log(String),
    Arguments(serde_json::Value),
    Config {
        value: serde_json::Value,
        name: String,
    },
    InputUsed(InputUsed),
    Error(String),
    ExperimentComplete(ExperimentCompletion),
}
