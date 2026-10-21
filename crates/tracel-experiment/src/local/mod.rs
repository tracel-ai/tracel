//! Recording experiments on this machine, without reaching any server.

mod artifacts;
mod events;
mod runs;
mod status;
#[cfg(test)]
mod test_support;

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::mem;
use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
use crate::{ArtifactKind, CancelToken, ExperimentProvider, ExperimentRun};
use artifacts::{LocalExperimentReader, save_artifact};
use events::{EVENTS_FILE, Line, lines, to_jsonl};
use runs::{child_dir, create_local_run_dir, create_runs_dir};
use status::{STATUS_FILE, Status};

/// Records experiments under a directory on this machine.
///
/// The runs of the experiment named `name` are numbered from 1 under `<dir>/<name>`, so `name`,
/// read as a relative path, is one directory name: one that is empty, `.`, `..` or a path of
/// several is refused. Each run keeps its status in `status.json`, written when the run starts and
/// again when it ends, appends its events to `events.jsonl`, and saves its artifacts under
/// `artifacts/`, each in the directory its name gives there, read as a relative path, such as
/// `checkpoints/best`; a name that resolves to `artifacts/` itself or outside it, such as `..`, is
/// refused. Creating a `LocalExperiments` performs no I/O; the first run creates the directories.
/// When it creates `dir` itself, it writes a `.gitignore` there that ignores everything under it,
/// so the records stay out of git and of code packages; a `dir` that already exists is left as it
/// is.
///
/// `status.json` names the experiment and numbers the run, says whether the run is `running`,
/// `completed` or `failed`, when it started and ended, in RFC 3339 UTC to the second, and why it
/// failed:
///
/// ```json
/// {
///   "name": "mnist",
///   "num": 3,
///   "status": "failed",
///   "started_at": "2026-10-06T14:02:11Z",
///   "finished_at": "2026-10-06T14:31:40Z",
///   "error": "loss is NaN"
/// }
/// ```
///
/// Each line of `events.jsonl` is one event, a JSON object whose `type` names it, with the
/// fields the console's experiment API gives it:
///
/// | `type` | Fields |
/// | --- | --- |
/// | `attribute` | `key`, `value` |
/// | `arguments` | `value`, the run's input, which the console lists as the experiment's `config`; the last one is the run's |
/// | `config` | `name`, `value` |
/// | `log` | `timestamp`, `level`, `message`, `metadata`, `activity` |
/// | `metric` | `name`, `epoch`, `iteration`, `value`, `group`, `activity` |
/// | `metric_definition` | `name`, `description`, `unit`, `higher_is_better` |
/// | `epoch_summary` | `name`, `epoch`, `group`, `value`, `activity` |
/// | `summary` | `name`, `value`, `activity` |
/// | `input_used` | `experiment`, `name`, `artifact_id` |
/// | `activity_started` | `activity`: `id`, `parent`, `name`, `cancellable`, `meter`, `attributes` |
/// | `activity_updated` | `id`, `current` |
/// | `activity_message` | `id`, `message` |
/// | `activity_finished` | `id`, `status`, `message` |
///
/// `activity` is the ID of the activity an event was recorded in, or `null`. A metric logged with
/// several values is one `metric` line per value.
///
/// ```
/// use std::sync::Arc;
///
/// use tracel_experiment::Experiments;
/// use tracel_experiment::local::LocalExperiments;
///
/// let experiments = Experiments::new(Arc::new(LocalExperiments::new("./runs")));
/// ```
#[derive(Debug, Clone)]
pub struct LocalExperiments {
    dir: PathBuf,
}

impl LocalExperiments {
    /// Records experiments under `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl ExperimentProvider for LocalExperiments {
    fn create_experiment(
        &self,
        name: String,
        attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError> {
        let root = child_dir(&self.dir, &name).ok_or_else(|| {
            ExperimentError::new(
                ExperimentErrorKind::Internal,
                format!("Invalid local experiment name: {name:?}"),
            )
        })?;
        let internal = |message: &'static str| {
            move |error| ExperimentError::with_source(ExperimentErrorKind::Internal, message, error)
        };
        create_runs_dir(&self.dir)
            .map_err(internal("Failed to create local experiment directory"))?;
        let root = root
            .canonicalize()
            .or_else(|_| {
                fs::create_dir_all(&root)?;
                root.canonicalize()
            })
            .map_err(internal("Failed to create local experiment directory"))?;
        let (num, run_root) = create_local_run_dir(&root)
            .map_err(internal("Failed to create local experiment run directory"))?;
        let session = LocalExperimentSession::start(run_root, name, num, attributes)
            .map_err(internal("Failed to initialize local experiment session"))?;
        let reader = LocalExperimentReader { root };

        let run = ExperimentRun::new(num.to_string(), session, reader, CancelToken::new());
        Ok(run)
    }
}

struct LocalExperimentSession {
    root: PathBuf,
    status: Status,
    events: Mutex<Events>,
}

/// Where a run's events go: appended to its events file, until a write fails or the run finishes.
enum Events {
    Recording(BufWriter<Box<dyn Write + Send>>),
    Failed(io::Error),
    Finished,
}

impl LocalExperimentSession {
    /// Starts recording the run `num` of the experiment `name` in `root`: writes its status as
    /// running and its attributes as its first events.
    fn start(
        root: PathBuf,
        name: String,
        num: u64,
        attributes: HashMap<String, Value>,
    ) -> io::Result<Self> {
        fs::create_dir_all(root.join("artifacts"))?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(EVENTS_FILE))?;
        Self::start_writing(root, Box::new(file), name, num, attributes)
    }

    /// Starts recording the run as [`start`](Self::start) does, with its events written to
    /// `events`.
    fn start_writing(
        root: PathBuf,
        events: Box<dyn Write + Send>,
        name: String,
        num: u64,
        attributes: HashMap<String, Value>,
    ) -> io::Result<Self> {
        let mut attributes: Vec<(String, Value)> = attributes.into_iter().collect();
        attributes.sort_by(|(a, _), (b, _)| a.cmp(b));
        let attributes = attributes
            .into_iter()
            .map(|(key, value)| Line::Attribute { key, value });
        let mut events = BufWriter::new(events);
        events.write_all(to_jsonl(attributes)?.as_bytes())?;
        let status = Status::running(name, num);
        status.write(&root.join(STATUS_FILE))?;

        Ok(Self {
            root,
            status,
            events: Mutex::new(Events::Recording(events)),
        })
    }

    /// Runs `write` on the run's events file, unless the run has finished or a write has failed:
    /// the first write that fails ends the recording.
    fn write_events(
        &self,
        write: impl FnOnce(&mut BufWriter<Box<dyn Write + Send>>) -> io::Result<()>,
    ) -> Result<(), ExperimentError> {
        let mut events = self.events.lock().unwrap();
        let file = match &mut *events {
            Events::Recording(file) => file,
            Events::Failed(error) => return Err(writer_failed(error)),
            Events::Finished => return Err(already_finished()),
        };
        write(file).map_err(|error| {
            let failure = writer_failed(&error);
            *events = Events::Failed(error);
            failure
        })
    }
}

impl ExperimentSession for LocalExperimentSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        let lines = to_jsonl(lines(event)).map_err(|err| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to serialize local experiment event",
                err,
            )
        })?;
        self.write_events(|events| events.write_all(lines.as_bytes()))
    }

    fn flush(&self) -> Result<(), ExperimentError> {
        self.write_events(|events| events.flush())
    }

    fn save_artifact(
        &self,
        name: &str,
        _kind: ArtifactKind,
        artifact: Box<BundleFn>,
    ) -> Result<(), ExperimentError> {
        save_artifact(&self.root, name, artifact)
    }

    /// Writes every event recorded, then the run's status: failed, with the writer's error, when
    /// an event could not be written.
    fn finish(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        let events = mem::replace(&mut *self.events.lock().unwrap(), Events::Finished);
        let written = match events {
            Events::Recording(mut file) => file.flush().map_err(|error| writer_failed(&error)),
            Events::Failed(error) => Err(writer_failed(&error)),
            Events::Finished => return Err(already_finished()),
        };
        let completion = match &written {
            Ok(()) => completion,
            Err(error) => ExperimentCompletion::Failed(error.message.clone()),
        };

        self.status
            .ended(completion)
            .write(&self.root.join(STATUS_FILE))
            .map_err(|err| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Internal,
                    "Failed to write the local experiment status",
                    err,
                )
            })?;
        written
    }
}

fn already_finished() -> ExperimentError {
    ExperimentError::new(
        ExperimentErrorKind::AlreadyFinished,
        "Local experiment session has already finished",
    )
}

fn writer_failed(error: &io::Error) -> ExperimentError {
    ExperimentError::new(
        ExperimentErrorKind::Internal,
        format!("Local experiment writer failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::MetricValue;
    use status::StatusKind;
    use test_support::{EXPERIMENT, LocalRun, local_run, read_json, read_lines, run_dir};

    #[test]
    fn an_experiment_name_that_is_not_one_directory_creates_nothing() {
        let dir = tempfile::tempdir().unwrap();

        LocalExperiments::new(dir.path().join("runs"))
            .create_experiment("..".to_string(), HashMap::new())
            .err()
            .expect("the name is refused");

        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_run_records_its_attributes_sorted_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = LocalExperiments::new(dir.path())
            .create_experiment(
                EXPERIMENT.to_string(),
                HashMap::from([
                    ("seed".to_string(), json!(42)),
                    ("kind".to_string(), json!("example")),
                    ("dataset".to_string(), json!("mnist")),
                ]),
            )
            .unwrap();

        experiment.handle().flush();

        assert_eq!(
            read_lines(&run_dir(dir.path(), &experiment).join(EVENTS_FILE)),
            [
                json!({"type": "attribute", "key": "dataset", "value": "mnist"}),
                json!({"type": "attribute", "key": "kind", "value": "example"}),
                json!({"type": "attribute", "key": "seed", "value": 42}),
            ]
        );
    }

    #[rstest]
    fn flush_writes_the_events_recorded_before_it_returns(local_run: LocalRun) {
        local_run.run.log_summary(vec![MetricValue {
            name: "accuracy".to_string(),
            value: 0.95,
        }]);

        local_run.run.handle().flush();

        assert_eq!(read_lines(&local_run.dir.join(EVENTS_FILE)).len(), 1);
    }

    #[rstest]
    fn a_finished_run_is_completed_in_its_status(local_run: LocalRun) {
        local_run.run.finish().unwrap();

        let status = read_json(&local_run.dir.join(STATUS_FILE));
        assert_eq!(status["status"], json!(StatusKind::Completed));
    }

    #[derive(Default)]
    struct FailsFirstWrite {
        failed: bool,
    }

    impl Write for FailsFirstWrite {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.failed {
                return Ok(buf.len());
            }
            self.failed = true;
            Err(io::Error::other("no space left on device"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailedWrite {
        session: LocalExperimentSession,
        root: TempDir,
    }

    #[fixture]
    fn failed_write() -> FailedWrite {
        let root = tempfile::tempdir().unwrap();
        let session = LocalExperimentSession::start_writing(
            root.path().to_path_buf(),
            Box::new(FailsFirstWrite::default()),
            EXPERIMENT.to_string(),
            1,
            HashMap::new(),
        )
        .unwrap();
        session
            .record_event(Event::Args(json!({"epochs": 2})))
            .unwrap();
        session.flush().unwrap_err();
        FailedWrite { session, root }
    }

    #[rstest]
    fn a_run_whose_events_failed_to_write_ends_as_failed(failed_write: FailedWrite) {
        let _ = failed_write.session.finish(ExperimentCompletion::Success);

        let status = read_json(&failed_write.root.path().join(STATUS_FILE));
        assert_eq!(status["status"], json!(StatusKind::Failed));
    }

    #[rstest]
    fn finishing_a_run_whose_events_failed_to_write_reports_the_failure(failed_write: FailedWrite) {
        let finished = failed_write.session.finish(ExperimentCompletion::Success);

        assert_eq!(finished.unwrap_err().kind, ExperimentErrorKind::Internal);
    }
}
