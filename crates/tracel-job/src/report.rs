//! The run report: how a job is going and how it ended, and the experiment it recorded, if any.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PROTOCOL, json_file};

/// Names the file the runner of a job writes the job's [`RunReport`] to.
pub const TRACEL_REPORT_FILE: &str = "TRACEL_REPORT_FILE";

/// How a job is going and how it ended, and the experiment it recorded, if any, as JSON.
///
/// It is the contents of the file [`TRACEL_REPORT_FILE`] names. The runner of a job writes it when
/// the job starts, with the status `running`, again when the job records an experiment, and again
/// when the job ends. The job does not own the experiment: the report links it, as the
/// experiment's `tracel.job_num` attribute links the job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunReport {
    /// The [`PROTOCOL`] version the report follows.
    pub protocol: u32,
    /// The name of the job.
    pub job: String,
    /// How the job is going.
    pub status: RunStatus,
    /// When the job started, in RFC 3339 UTC to the second.
    pub started_at: String,
    /// When the job ended, in RFC 3339 UTC to the second; `None` while it runs.
    pub finished_at: Option<String>,
    /// Why the job failed; `None` unless it failed.
    pub error: Option<String>,
    /// The experiment the job recorded; `None` until it records one, and for a job that records
    /// none, such as an inference.
    #[serde(default)]
    pub experiment: Option<ReportedExperiment>,
}

/// The experiment a job recorded, as its [`RunReport`] links it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedExperiment {
    /// The experiment's number, when its ID is a number, as it is for every backend of the SDK.
    pub num: Option<u64>,
    /// The experiment's page on the console; `None` offline.
    pub url: Option<String>,
    /// The directory an offline experiment is recorded in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
}

/// How a job is going, in a [`RunReport`].
///
/// A job ends as completed or failed by what it returns, whether or not it was asked to stop:
/// being stopped is known to whoever stopped it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The job has not ended.
    Running,
    /// The job succeeded.
    Completed,
    /// The job failed, for the reason in [`RunReport::error`].
    Failed,
}

impl RunReport {
    /// The report of `job`, started at `started_at`: running, with no experiment yet, and
    /// following [`PROTOCOL`].
    pub fn new(job: impl Into<String>, started_at: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL,
            job: job.into(),
            status: RunStatus::Running,
            started_at: started_at.into(),
            finished_at: None,
            error: None,
            experiment: None,
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
            experiment: Some(console_experiment()),
            ..RunReport::new("mnist", STARTED_AT)
        };

        report.write(&path).unwrap();

        let expected = r#"{
  "protocol": 1,
  "job": "mnist",
  "status": "failed",
  "started_at": "2026-10-06T14:02:11Z",
  "finished_at": "2026-10-06T14:31:40Z",
  "error": "out of memory",
  "experiment": {
    "num": 42,
    "url": "https://console.tracel.ai/users/me/projects/mnist/experiments/42"
  }
}
"#;
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(RunReport::read(&path).unwrap(), report);
    }

    #[test]
    fn a_new_report_is_running_with_no_experiment() {
        let report = RunReport::new("mnist", STARTED_AT);

        assert_eq!(report.protocol, PROTOCOL);
        assert_eq!(report.job, "mnist");
        assert_eq!(report.status, RunStatus::Running);
        assert_eq!(report.started_at, STARTED_AT);
        assert_eq!(report.finished_at, None);
        assert_eq!(report.error, None);
        assert_eq!(report.experiment, None);
    }

    #[test]
    fn a_report_without_an_experiment_gives_it_as_null() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        RunReport::new("wordtok", STARTED_AT).write(&path).unwrap();

        let fields: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(fields["experiment"], serde_json::Value::Null);
        assert!(fields.as_object().unwrap().contains_key("experiment"));
    }

    #[test]
    fn a_report_that_leaves_out_the_experiment_reads_as_having_none() {
        let report: RunReport = serde_json::from_str(
            r#"{"protocol": 1, "job": "wordtok", "status": "completed",
                "started_at": "2026-10-06T14:02:11Z", "finished_at": "2026-10-06T14:02:12Z",
                "error": null}"#,
        )
        .unwrap();

        assert_eq!(report.experiment, None);
        assert_eq!(report.status, RunStatus::Completed);
    }

    #[test]
    fn a_status_is_running_completed_or_failed() {
        for (status, name) in [
            (RunStatus::Running, "running"),
            (RunStatus::Completed, "completed"),
            (RunStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), name);
            assert_eq!(
                serde_json::from_value::<RunStatus>(name.into()).unwrap(),
                status
            );
        }
        assert!(serde_json::from_value::<RunStatus>("cancelled".into()).is_err());
    }

    #[test]
    fn an_offline_experiment_gives_its_directory_and_no_url() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let experiment = ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some(dir.path().join("mnist/3")),
        };

        RunReport {
            experiment: Some(experiment.clone()),
            ..RunReport::new("mnist", STARTED_AT)
        }
        .write(&path)
        .unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let report: RunReport = serde_json::from_str(&contents).unwrap();
        let fields: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(report.experiment, Some(experiment));
        assert_eq!(fields["experiment"]["url"], serde_json::Value::Null);
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn the_report_replaces_the_file_through_a_temporary_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        std::fs::write(&path, "stale").unwrap();
        let running = RunReport::new("mnist", STARTED_AT);

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

        let report = RunReport::new("mnist", STARTED_AT);

        assert!(report.write(&path).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
