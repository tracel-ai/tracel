use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use super::LocalExperiments;
use crate::{ExperimentProvider, ExperimentRun};

pub fn run(dir: &Path, name: &str) -> ExperimentRun {
    LocalExperiments::new(dir)
        .create_experiment(name.to_string(), HashMap::new())
        .unwrap()
}

pub fn read_lines(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

pub fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
