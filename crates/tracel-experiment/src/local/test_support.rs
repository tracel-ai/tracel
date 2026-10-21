use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rstest::fixture;
use serde_json::Value;
use tempfile::TempDir;

use super::LocalExperiments;
use crate::{ExperimentProvider, ExperimentRun};

pub const EXPERIMENT: &str = "mnist";

pub struct LocalRun {
    pub run: ExperimentRun,
    pub dir: PathBuf,
    pub runs: TempDir,
}

#[fixture]
pub fn local_run() -> LocalRun {
    let runs = tempfile::tempdir().unwrap();
    let run = run(runs.path(), EXPERIMENT);
    LocalRun {
        dir: run_dir(runs.path(), &run),
        run,
        runs,
    }
}

pub fn run(dir: &Path, name: &str) -> ExperimentRun {
    LocalExperiments::new(dir)
        .create_experiment(name.to_string(), HashMap::new())
        .unwrap()
}

pub fn run_dir(runs: &Path, run: &ExperimentRun) -> PathBuf {
    runs.join(EXPERIMENT).join(run.id().as_str())
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
