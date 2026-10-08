//! The run report: how a job is going and how it ended, and the experiments it started.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::PROTOCOL;
use crate::json::{self, ParseError};

/// Names the file the runner of a job writes the job's [`RunReport`] to.
pub const TRACEL_REPORT_FILE: &str = "TRACEL_REPORT_FILE";

/// Gives the number a launcher gave the job it runs, which each experiment the job starts records
/// as its [`JOB_NUM_ATTRIBUTE`].
pub const TRACEL_JOB_NUM: &str = "TRACEL_JOB_NUM";

/// The experiment attribute that links an experiment to the job that started it: the job's
/// [`TRACEL_JOB_NUM`], as a number when it is one.
pub const JOB_NUM_ATTRIBUTE: &str = "tracel.job_num";

/// How a job is going and how it ended, and the experiments it started, as JSON.
///
/// It is the contents of the file [`TRACEL_REPORT_FILE`] names. The runner of a job writes it when
/// the job starts, with the status `running`, again each time the job starts an experiment, and
/// again when the job ends. The job does not own its experiments: the report links them, as each
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
    /// The experiments the job started, in the order it started them; empty until it starts one.
    #[serde(default)]
    pub experiments: Vec<ReportedExperiment>,
}

/// An experiment a job started, as its [`RunReport`] links it.
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
    /// The report of `job`, started at `started_at`: running, with no experiments yet, and
    /// following [`PROTOCOL`].
    pub fn new(job: impl Into<String>, started_at: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL,
            job: job.into(),
            status: RunStatus::Running,
            started_at: started_at.into(),
            finished_at: None,
            error: None,
            experiments: Vec::new(),
        }
    }

    /// The report's file contents: pretty-printed JSON ending with a newline.
    pub fn to_json(&self) -> String {
        json::to_json(self)
    }

    /// The report `json` holds.
    ///
    /// Fails with [`ParseError::Protocol`] when it follows another [`PROTOCOL`] version, and with
    /// [`ParseError::Invalid`] when it is not a run report.
    pub fn from_json(json: &[u8]) -> Result<Self, ParseError> {
        json::from_json(json)
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
        let report = RunReport {
            status: RunStatus::Failed,
            finished_at: Some("2026-10-06T14:31:40Z".into()),
            error: Some("out of memory".into()),
            experiments: vec![console_experiment()],
            ..RunReport::new("mnist", STARTED_AT)
        };

        let expected = r#"{
  "protocol": 1,
  "job": "mnist",
  "status": "failed",
  "started_at": "2026-10-06T14:02:11Z",
  "finished_at": "2026-10-06T14:31:40Z",
  "error": "out of memory",
  "experiments": [
    {
      "num": 42,
      "url": "https://console.tracel.ai/users/me/projects/mnist/experiments/42"
    }
  ]
}
"#;
        assert_eq!(report.to_json(), expected);
        assert_eq!(RunReport::from_json(expected.as_bytes()).unwrap(), report);
    }

    #[test]
    fn a_new_report_is_running_with_no_experiments() {
        let report = RunReport::new("mnist", STARTED_AT);

        assert_eq!(report.protocol, PROTOCOL);
        assert_eq!(report.job, "mnist");
        assert_eq!(report.status, RunStatus::Running);
        assert_eq!(report.started_at, STARTED_AT);
        assert_eq!(report.finished_at, None);
        assert_eq!(report.error, None);
        assert!(report.experiments.is_empty());
    }

    #[test]
    fn a_report_without_experiments_gives_them_as_an_empty_list() {
        let json = RunReport::new("evaluate", STARTED_AT).to_json();

        let fields: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(fields["experiments"], serde_json::json!([]));
    }

    #[test]
    fn a_report_that_leaves_out_the_experiments_reads_as_having_none() {
        let report = RunReport::from_json(
            br#"{"protocol": 1, "job": "evaluate", "status": "completed",
                "started_at": "2026-10-06T14:02:11Z", "finished_at": "2026-10-06T14:02:12Z",
                "error": null}"#,
        )
        .unwrap();

        assert!(report.experiments.is_empty());
        assert_eq!(report.status, RunStatus::Completed);
    }

    #[test]
    fn only_a_run_report_of_this_protocol_parses() {
        let incomplete = RunReport::from_json(br#"{"protocol": 1, "job": "evaluate"}"#);
        let other = RunReport::from_json(br#"{"protocol": 2, "state": "done"}"#);

        assert!(matches!(incomplete, Err(ParseError::Invalid(_))));
        assert!(matches!(other, Err(ParseError::Protocol { found: 2 })));
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
        let experiment = ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some("/home/me/runs/mnist/3".into()),
        };

        let json = RunReport {
            experiments: vec![experiment.clone()],
            ..RunReport::new("mnist", STARTED_AT)
        }
        .to_json();

        let report = RunReport::from_json(json.as_bytes()).unwrap();
        let fields: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(report.experiments, [experiment]);
        assert_eq!(fields["experiments"][0]["url"], serde_json::Value::Null);
        assert_eq!(fields["experiments"][0]["dir"], "/home/me/runs/mnist/3");
    }
}
