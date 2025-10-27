use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputUsed {
    Artifact { artifact_id: String },
    Model { model_version_id: String },
}

#[derive(Debug, Serialize, Clone)]
pub enum ExperimentMessage {
    MetricLog {
        name: String,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: String,
    },
    Log(String),
    InputUsed(InputUsed),
    Error(String),
}
