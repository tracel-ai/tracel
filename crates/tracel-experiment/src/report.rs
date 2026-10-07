//! The run report a run keeps at a path: written when the run starts, and again when it ends.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use tracel_job::{ReportedExperiment, RunReport, RunStatus};

use crate::session::ExperimentCompletion;

/// A run report kept at a path: written when the run starts, and again when it ends.
pub struct ReportFile {
    path: PathBuf,
    report: RunReport,
}

impl ReportFile {
    /// Writes to `path` the report of a run of `job`, started now, that created `experiment`.
    pub fn start(path: PathBuf, job: &str, experiment: ReportedExperiment) -> io::Result<Self> {
        let report = RunReport::new(job, experiment, now());
        report.write(&path)?;
        Ok(Self { path, report })
    }

    /// Rewrites the report as the run ended now, with `completion`.
    pub fn finish(&self, completion: &ExperimentCompletion) -> io::Result<()> {
        let (status, error) = match completion {
            ExperimentCompletion::Success => (RunStatus::Completed, None),
            ExperimentCompletion::Failed(reason) => (RunStatus::Failed, Some(reason.clone())),
            ExperimentCompletion::Cancelled => (RunStatus::Cancelled, None),
        };
        RunReport {
            status,
            finished_at: Some(now()),
            error,
            ..self.report.clone()
        }
        .write(&self.path)
    }

    /// The path the report is kept at.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The current time, in RFC 3339 UTC to the second.
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn console_experiment() -> ReportedExperiment {
        ReportedExperiment {
            num: Some(42),
            url: Some("https://console.tracel.ai/users/me/projects/mnist/experiments/42".into()),
            dir: None,
        }
    }

    fn is_utc_seconds(timestamp: &Value) -> bool {
        timestamp.as_str().is_some_and(|timestamp| {
            timestamp.ends_with('Z')
                && !timestamp.contains('.')
                && chrono::DateTime::parse_from_rfc3339(timestamp).is_ok()
        })
    }

    #[test]
    fn a_started_run_is_reported_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        let file = ReportFile::start(path.clone(), "mnist", console_experiment()).unwrap();

        assert_eq!(file.path(), path.as_path());
        let mut report = read(&path);
        assert!(is_utc_seconds(&report["started_at"]), "{report}");
        report["started_at"] = Value::Null;
        assert_eq!(
            report,
            json!({
                "protocol": 1,
                "job": "mnist",
                "experiment": {
                    "num": 42,
                    "url": "https://console.tracel.ai/users/me/projects/mnist/experiments/42"
                },
                "status": "running",
                "started_at": null,
                "finished_at": null,
                "error": null
            })
        );
    }

    #[test]
    fn an_ended_run_is_reported_with_its_status_and_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let file = ReportFile::start(path.clone(), "mnist", console_experiment()).unwrap();
        let started_at = read(&path)["started_at"].clone();

        for (completion, status, error) in [
            (ExperimentCompletion::Success, "completed", Value::Null),
            (
                ExperimentCompletion::Failed("out of memory".into()),
                "failed",
                json!("out of memory"),
            ),
            (ExperimentCompletion::Cancelled, "cancelled", Value::Null),
        ] {
            file.finish(&completion).unwrap();

            let report = read(&path);
            assert_eq!(report["status"], status);
            assert_eq!(report["error"], error);
            assert_eq!(report["started_at"], started_at);
            assert!(is_utc_seconds(&report["finished_at"]), "{report}");
            assert_eq!(report["experiment"]["num"], 42);
        }
    }
}
