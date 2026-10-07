//! The run report of the job the command line runs, kept at the path `TRACEL_REPORT_FILE` names.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::{SecondsFormat, Utc};
use tracel_job::{RunReport, RunStatus, TRACEL_REPORT_FILE};

use crate::{BoxError, ExperimentReporter};

/// The path `TRACEL_REPORT_FILE` names, when it names one.
pub fn path_from_env() -> Option<PathBuf> {
    path_from_vars(|name| std::env::var_os(name))
}

/// [`path_from_env`] with the environment variables `lookup` gives.
fn path_from_vars(lookup: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    lookup(TRACEL_REPORT_FILE)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

/// The run report of a job, kept at a path: written when the job starts, again when it records an
/// experiment, and again when it ends.
///
/// It is the report's one writer: the reporter it gives the job rewrites the report through it.
pub struct ReportFile {
    kept: Arc<Kept>,
}

/// A report and the path it is kept at, shared with the reporter the job is given.
struct Kept {
    path: PathBuf,
    report: Mutex<RunReport>,
}

impl Kept {
    /// Changes the report with `change`, then rewrites it.
    fn rewrite(&self, change: impl FnOnce(&mut RunReport)) -> Result<(), ReportError> {
        let mut report = self.report.lock().unwrap();
        change(&mut report);
        report.write(&self.path).map_err(|source| ReportError {
            path: self.path.clone(),
            source,
        })
    }
}

impl ReportFile {
    /// Writes to `path` the report of `job`, started now: running, with no experiment.
    pub fn start(path: PathBuf, job: &str) -> Result<Self, ReportError> {
        let report = RunReport::new(job, now());
        if let Err(source) = report.write(&path) {
            return Err(ReportError { path, source });
        }
        Ok(Self {
            kept: Arc::new(Kept {
                path,
                report: Mutex::new(report),
            }),
        })
    }

    /// The reporter the job is given: it links the experiment the job records to the report and
    /// rewrites it, and once the job has ended, does nothing.
    pub fn experiment_reporter(&self) -> ExperimentReporter {
        let kept = Arc::downgrade(&self.kept);
        ExperimentReporter::new(move |experiment| {
            let Some(kept) = kept.upgrade() else {
                return;
            };
            if let Err(error) = kept.rewrite(|report| report.experiment = Some(experiment)) {
                eprintln!("warning: {error}");
            }
        })
    }

    /// Rewrites the report as the job ended now with `outcome`: completed, or failed with its
    /// error.
    pub fn finish(self, outcome: &Result<(), BoxError>) -> Result<(), ReportError> {
        let (status, error) = match outcome {
            Ok(()) => (RunStatus::Completed, None),
            Err(error) => (RunStatus::Failed, Some(error.to_string())),
        };
        self.kept.rewrite(|report| {
            report.status = status;
            report.finished_at = Some(now());
            report.error = error;
        })
    }
}

/// The run report could not be written.
#[derive(Debug, thiserror::Error)]
#[error("failed to write the run report to {}: {source}", path.display())]
pub struct ReportError {
    path: PathBuf,
    source: io::Error,
}

/// The current time, in RFC 3339 UTC to the second.
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};
    use tracel_job::ReportedExperiment;

    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    fn is_utc_seconds(timestamp: &Value) -> bool {
        timestamp.as_str().is_some_and(|timestamp| {
            timestamp.ends_with('Z')
                && !timestamp.contains('.')
                && chrono::DateTime::parse_from_rfc3339(timestamp).is_ok()
        })
    }

    #[test]
    fn the_path_is_the_one_tracel_report_file_names() {
        let lookup = |value: Option<&'static str>| {
            move |name: &str| {
                assert_eq!(name, TRACEL_REPORT_FILE);
                value.map(OsString::from)
            }
        };

        assert_eq!(
            path_from_vars(lookup(Some("report.json"))),
            Some(PathBuf::from("report.json"))
        );
        assert_eq!(path_from_vars(lookup(Some(""))), None);
        assert_eq!(path_from_vars(lookup(None)), None);
    }

    #[test]
    fn a_started_job_is_reported_running_with_no_experiment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        let _report = ReportFile::start(path.clone(), "mnist").unwrap();

        let mut report = read(&path);
        assert!(is_utc_seconds(&report["started_at"]), "{report}");
        report["started_at"] = Value::Null;
        assert_eq!(
            report,
            json!({
                "protocol": 1,
                "job": "mnist",
                "status": "running",
                "started_at": null,
                "finished_at": null,
                "error": null,
                "experiment": null
            })
        );
    }

    #[test]
    fn the_experiment_the_job_records_is_linked_until_the_job_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");
        let report = ReportFile::start(path.clone(), "mnist").unwrap();
        let reporter = report.experiment_reporter();
        let experiment = ReportedExperiment {
            num: Some(3),
            url: None,
            dir: Some("runs/mnist/3".into()),
        };

        reporter.report(experiment);
        let linked = read(&path);
        report.finish(&Err("loss is NaN".into())).unwrap();
        let failed = read(&path);
        reporter.report(ReportedExperiment {
            num: Some(4),
            url: None,
            dir: None,
        });

        assert_eq!(linked["status"], "running");
        assert_eq!(
            linked["experiment"],
            json!({"num": 3, "url": null, "dir": "runs/mnist/3"})
        );
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"], "loss is NaN");
        assert_eq!(failed["started_at"], linked["started_at"]);
        assert!(is_utc_seconds(&failed["finished_at"]), "{failed}");
        assert_eq!(failed["experiment"], linked["experiment"]);
        assert_eq!(read(&path), failed);
    }

    #[test]
    fn a_job_that_succeeds_is_reported_completed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.json");

        ReportFile::start(path.clone(), "wordtok")
            .unwrap()
            .finish(&Ok(()))
            .unwrap();

        let report = read(&path);
        assert_eq!(report["status"], "completed");
        assert_eq!(report["error"], Value::Null);
        assert_eq!(report["experiment"], Value::Null);
        assert!(is_utc_seconds(&report["finished_at"]), "{report}");
    }

    #[test]
    fn a_report_that_cannot_be_written_names_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("report.json");

        let error = ReportFile::start(path.clone(), "mnist").err().unwrap();

        assert!(
            error.to_string().starts_with(&format!(
                "failed to write the run report to {}: ",
                path.display()
            )),
            "{error}"
        );
    }
}
