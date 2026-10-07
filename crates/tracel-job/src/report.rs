//! The run report: the experiment a run created, and how the run went.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PROTOCOL, json_file};

/// Names the file an experiment run writes its [`RunReport`] to.
pub const TRACEL_REPORT_FILE: &str = "TRACEL_REPORT_FILE";

/// The experiment a run created and how the run went, as JSON.
///
/// It is the contents of the file [`TRACEL_REPORT_FILE`] names, and of an offline run's
/// `status.json`. It is written when the experiment is created, with the status `running`, and
/// again when the run ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// The [`PROTOCOL`] version the report follows.
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

impl RunReport {
    /// The report of a run of `job`, started at `started_at`, that created `experiment`: running,
    /// and following [`PROTOCOL`].
    pub fn new(
        job: impl Into<String>,
        experiment: ReportedExperiment,
        started_at: impl Into<String>,
    ) -> Self {
        Self {
            protocol: PROTOCOL,
            job: job.into(),
            experiment,
            status: RunStatus::Running,
            started_at: started_at.into(),
            finished_at: None,
            error: None,
        }
    }

    /// Writes the report to `path` atomically: to `<path>.tmp` first, then renamed to `path`.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        json_file::write(path, self)
    }

    /// Reads the report at `path`.
    ///
    /// Fails when it cannot be read or is not a run report. It is read whatever its `protocol`,
    /// which the caller compares with [`PROTOCOL`].
    pub fn read(path: &Path) -> io::Result<Self> {
        json_file::read(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTED_AT: &str = "2026-10-06T14:02:11Z";

    fn console_experiment() -> ReportedExperiment {
        ReportedExperiment {
            num: Some(42),
            url: Some("https://console.tracel.ai/users/me/projects/mnist/experiments/42".into()),
            dir: None,
        }
    }

    #[test]
    fn the_report_is_pretty_printed_json_ending_with_a_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = RunReport {
            status: RunStatus::Failed,
            finished_at: Some("2026-10-06T14:31:40Z".into()),
            error: Some("out of memory".into()),
            ..RunReport::new("mnist", console_experiment(), STARTED_AT)
        };

        report.write(&path).unwrap();

        let expected = r#"{
  "protocol": 1,
  "job": "mnist",
  "experiment": {
    "num": 42,
    "url": "https://console.tracel.ai/users/me/projects/mnist/experiments/42"
  },
  "status": "failed",
  "started_at": "2026-10-06T14:02:11Z",
  "finished_at": "2026-10-06T14:31:40Z",
  "error": "out of memory"
}
"#;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(RunReport::read(&path).unwrap(), report);
    }

    #[test]
    fn a_new_report_is_running() {
        let report = RunReport::new("mnist", console_experiment(), STARTED_AT);

        assert_eq!(report.protocol, PROTOCOL);
        assert_eq!(report.status, RunStatus::Running);
        assert_eq!(report.started_at, STARTED_AT);
        assert_eq!(report.finished_at, None);
        assert_eq!(report.error, None);
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

        RunReport::new("mnist", experiment.clone(), STARTED_AT)
            .write(&path)
            .unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let report: RunReport = serde_json::from_str(&contents).unwrap();
        let fields: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(report.experiment, experiment);
        assert_eq!(report.status, RunStatus::Running);
        assert_eq!(fields["experiment"]["url"], serde_json::Value::Null);
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn the_report_replaces_the_file_through_a_temporary_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, "stale").unwrap();
        let running = RunReport::new("mnist", console_experiment(), STARTED_AT);

        running.write(&path).unwrap();
        RunReport {
            status: RunStatus::Completed,
            ..running
        }
        .write(&path)
        .unwrap();

        assert_eq!(RunReport::read(&path).unwrap().status, RunStatus::Completed);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_report_that_cannot_be_written_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");

        let report = RunReport::new("mnist", console_experiment(), STARTED_AT);

        assert!(report.write(&path).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
