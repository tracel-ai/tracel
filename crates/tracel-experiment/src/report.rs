//! The run report: the experiment a run created, and how the run went.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::session::ExperimentCompletion;

/// The run report protocol version.
const PROTOCOL: u32 = 1;

/// The experiment a run created and how the run went, as JSON.
///
/// It is the contents of the file `TRACEL_REPORT_FILE` names, and of an offline run's
/// `status.json`. It is written when the experiment is created, with the status `running`, and
/// again when the run ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// The run report protocol version, `1`.
    pub protocol: u32,
    /// The name of the job that ran.
    pub job: String,
    /// The experiment the run created.
    pub experiment: ReportedExperiment,
    /// How the run is going.
    pub status: RunStatus,
    /// When the run started, in RFC 3339 UTC to the second.
    pub started_at: String,
    /// When the run ended, in RFC 3339 UTC to the second; `None` while it runs.
    pub finished_at: Option<String>,
    /// Why the run failed; `None` unless it failed.
    pub error: Option<String>,
}

/// The experiment a [`RunReport`] is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedExperiment {
    /// The experiment's number, when its ID is a number, as it is for every backend of the SDK.
    pub num: Option<u64>,
    /// The experiment's page on the console; `None` offline.
    pub url: Option<String>,
    /// The directory an offline run is recorded in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
}

/// How a run is going, in a [`RunReport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The run has not ended.
    Running,
    /// The run succeeded.
    Completed,
    /// The run failed, for the reason in [`RunReport::error`].
    Failed,
    /// The run was cancelled.
    Cancelled,
}

/// A run report kept at a path: written when the run starts, and again when it ends.
pub struct ReportFile {
    path: PathBuf,
    report: RunReport,
}

impl ReportFile {
    /// Writes to `path` the report of a run of `job`, started now, that created `experiment`.
    pub fn start(path: PathBuf, job: &str, experiment: ReportedExperiment) -> io::Result<Self> {
        let file = Self {
            path,
            report: RunReport {
                protocol: PROTOCOL,
                job: job.to_string(),
                experiment,
                status: RunStatus::Running,
                started_at: now(),
                finished_at: None,
                error: None,
            },
        };
        file.write(&file.report)?;
        Ok(file)
    }

    /// Rewrites the report as the run ended now, with `completion`.
    pub fn finish(&self, completion: &ExperimentCompletion) -> io::Result<()> {
        let (status, error) = match completion {
            ExperimentCompletion::Success => (RunStatus::Completed, None),
            ExperimentCompletion::Failed(reason) => (RunStatus::Failed, Some(reason.clone())),
            ExperimentCompletion::Cancelled => (RunStatus::Cancelled, None),
        };
        self.write(&RunReport {
            status,
            finished_at: Some(now()),
            error,
            ..self.report.clone()
        })
    }

    /// The path the report is kept at.
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, report: &RunReport) -> io::Result<()> {
        let mut contents = serde_json::to_vec_pretty(report)?;
        contents.push(b'\n');
        write_atomically(&self.path, &contents)
    }
}

/// Writes `contents` to `<path>.tmp`, then renames it to `path`, so `path` is never seen half
/// written.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let written = std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
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

        ReportFile::start(path.clone(), "mnist", console_experiment()).unwrap();

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

    #[test]
    fn an_offline_run_gives_its_directory_and_no_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let experiment = ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some(dir.path().join("mnist/3")),
        };

        ReportFile::start(path.clone(), "mnist", experiment.clone()).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let report: RunReport = serde_json::from_str(&contents).unwrap();
        assert_eq!(report.experiment, experiment);
        assert_eq!(report.status, RunStatus::Running);
        assert_eq!(read(&path)["experiment"]["url"], Value::Null);
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn the_report_replaces_the_file_through_a_temporary_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, "stale").unwrap();

        let file = ReportFile::start(path.clone(), "mnist", console_experiment()).unwrap();
        file.finish(&ExperimentCompletion::Success).unwrap();

        assert_eq!(read(&path)["status"], "completed");
        assert_eq!(file.path(), path.as_path());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_report_that_cannot_be_written_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");

        assert!(ReportFile::start(path, "mnist", console_experiment()).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
