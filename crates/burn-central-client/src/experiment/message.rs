use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputUsed {
    Artifact { artifact_id: String },
    Model { model_version_id: String },
}

#[derive(Debug, Serialize)]
pub enum ExperimentMessage {
    MetricLog {
        name: String,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: String,
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
    InputUsed(InputUsed),
    Error(String),
}
