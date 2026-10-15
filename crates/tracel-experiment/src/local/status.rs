use std::fs;
use std::io;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use serde::Serialize;

use crate::session::ExperimentCompletion;

/// The file a run keeps its [`Status`] in.
pub const STATUS_FILE: &str = "status.json";

/// How a run is going, as its `status.json` gives it.
#[derive(Clone, Serialize)]
pub struct Status {
    name: String,
    num: u64,
    status: StatusKind,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    Running,
    Completed,
    Failed,
}

impl Status {
    /// The status of the run `num` of the experiment `name`, started now.
    pub fn running(name: String, num: u64) -> Self {
        Self {
            name,
            num,
            status: StatusKind::Running,
            started_at: now(),
            finished_at: None,
            error: None,
        }
    }

    /// The status of the run once it ended now with `completion`.
    pub fn ended(&self, completion: ExperimentCompletion) -> Self {
        let (status, error) = match completion {
            ExperimentCompletion::Success => (StatusKind::Completed, None),
            ExperimentCompletion::Failed(reason) => (StatusKind::Failed, Some(reason)),
        };
        Self {
            status,
            finished_at: Some(now()),
            error,
            ..self.clone()
        }
    }

    /// Writes the status to `path` as pretty-printed JSON: to `<path>.tmp` first, then renamed to
    /// `path`, so it is never read half written.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut contents = serde_json::to_vec_pretty(self)?;
        contents.push(b'\n');
        let tmp = path.with_extension("json.tmp");
        let written = fs::write(&tmp, contents).and_then(|()| fs::rename(&tmp, path));
        if written.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        written
    }
}

/// The current time, in RFC 3339 UTC to the second.
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::{Value, json};

    use super::*;
    use crate::local::test_support::{EXPERIMENT, read_json, run};

    const REASON: &str = "loss is NaN";

    fn running() -> Status {
        Status::running(EXPERIMENT.to_string(), 1)
    }

    fn is_utc_seconds(timestamp: &Value) -> bool {
        timestamp.as_str().is_some_and(|timestamp| {
            timestamp.ends_with('Z')
                && !timestamp.contains('.')
                && chrono::DateTime::parse_from_rfc3339(timestamp).is_ok()
        })
    }

    #[test]
    fn the_status_follows_the_run_from_the_start_to_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = run(dir.path(), "mnist");
        let run_dir = dir.path().join("mnist/1");

        let mut running = read_json(&run_dir.join("status.json"));
        experiment.finish().unwrap();
        let mut completed = read_json(&run_dir.join("status.json"));

        assert!(is_utc_seconds(&running["started_at"]), "{running}");
        assert!(is_utc_seconds(&completed["finished_at"]), "{completed}");
        assert_eq!(completed["started_at"], running["started_at"]);
        running["started_at"] = Value::Null;
        completed["started_at"] = Value::Null;
        completed["finished_at"] = Value::Null;
        assert_eq!(
            running,
            json!({
                "name": "mnist",
                "num": 1,
                "status": "running",
                "started_at": null,
                "finished_at": null,
                "error": null
            })
        );
        assert_eq!(
            completed,
            json!({
                "name": "mnist",
                "num": 1,
                "status": "completed",
                "started_at": null,
                "finished_at": null,
                "error": null
            })
        );
        assert!(!run_dir.join("status.json.tmp").exists());
    }

    #[rstest]
    #[case::succeeded(ExperimentCompletion::Success, StatusKind::Completed, None)]
    #[case::failed(
        ExperimentCompletion::Failed(REASON.to_string()),
        StatusKind::Failed,
        Some(REASON)
    )]
    fn an_ended_status_says_how_the_run_ended(
        #[case] completion: ExperimentCompletion,
        #[case] status: StatusKind,
        #[case] error: Option<&str>,
    ) {
        let ended = running().ended(completion);

        assert_eq!((ended.status, ended.error.as_deref()), (status, error));
    }
}
