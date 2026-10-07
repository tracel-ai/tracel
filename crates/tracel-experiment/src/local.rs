//! Recording experiments on this machine, without reaching any server.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use tracel_artifact::bundle::FsBundle;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use crate::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
use crate::{
    ActivityEvent, ActivitySpec, ActivityStatus, ArtifactKind, CancelToken, ExperimentId,
    ExperimentLocation, ExperimentProvider, ExperimentRun, MetricSpec, MetricValue,
};

/// The file a run appends its events to, one JSON object per line.
const EVENTS_FILE: &str = "events.jsonl";
/// The file a run keeps its [`Status`] in.
const STATUS_FILE: &str = "status.json";

/// Records experiments under a directory on this machine.
///
/// The runs of the experiment named `name` are numbered from 1 under `<dir>/<name>`. Each run
/// keeps its status in `status.json`, written when the run starts and again when it ends, appends
/// its events to `events.jsonl`, and saves its artifacts under `artifacts/`. Creating a
/// `LocalExperiments` performs no I/O; the first run creates the directories.
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
/// | `arguments` | `value`, the run's input, which the console lists as the experiment's `config` |
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
        let internal = |message: &'static str| {
            move |error| ExperimentError::with_source(ExperimentErrorKind::Internal, message, error)
        };
        let root = self.dir.join(&name);
        let root = root
            .canonicalize()
            .or_else(|_| {
                fs::create_dir_all(&root)?;
                root.canonicalize()
            })
            .map_err(internal("Failed to create local experiment directory"))?;
        let (num, run_root) = create_local_run_dir(&root)
            .map_err(internal("Failed to create local experiment run directory"))?;
        let session = LocalExperimentSession::start(run_root.clone(), name, num, attributes)
            .map_err(internal("Failed to initialize local experiment session"))?;
        let reader = LocalExperimentReader { root };

        let run = ExperimentRun::new(num.to_string(), session, reader, CancelToken::new());
        Ok(run.with_location(ExperimentLocation::Dir(run_root)))
    }
}

struct LocalExperimentSession {
    root: PathBuf,
    status: Status,
    active: Mutex<Option<LocalWorker>>,
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
        let mut attributes: Vec<(String, Value)> = attributes.into_iter().collect();
        attributes.sort_by(|(a, _), (b, _)| a.cmp(b));
        let attributes = attributes
            .into_iter()
            .map(|(key, value)| Line::Attribute { key, value });
        let mut events = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join(EVENTS_FILE))?;
        events.write_all(to_jsonl(attributes)?.as_bytes())?;
        let status = Status::running(name, num);
        status.write(&root.join(STATUS_FILE))?;
        let (sender, receiver) = channel();
        let join = thread::spawn(move || local_worker(receiver, events));

        Ok(Self {
            root,
            status,
            active: Mutex::new(Some(LocalWorker { sender, join })),
        })
    }

    fn sender(&self) -> Result<Sender<LocalWrite>, ExperimentError> {
        let guard = self.active.lock().unwrap();
        guard
            .as_ref()
            .map(|worker| worker.sender.clone())
            .ok_or_else(|| {
                ExperimentError::new(
                    ExperimentErrorKind::AlreadyFinished,
                    "Local experiment session has already finished",
                )
            })
    }

    /// Stops the writer once it has written every event, then writes the run's status.
    fn finish_worker(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        let worker = self.active.lock().unwrap().take().ok_or_else(|| {
            ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Local experiment session has already finished",
            )
        })?;

        let send_result = worker.sender.send(LocalWrite::Finish);
        match worker.join.join() {
            Ok(Ok(())) if send_result.is_ok() => {}
            Ok(Ok(())) => {
                return Err(ExperimentError::new(
                    ExperimentErrorKind::Internal,
                    "Failed to send local experiment completion",
                ));
            }
            Ok(Err(err)) => {
                return Err(ExperimentError::with_source(
                    ExperimentErrorKind::Internal,
                    "Local experiment writer failed",
                    err,
                ));
            }
            Err(_) => {
                return Err(ExperimentError::new(
                    ExperimentErrorKind::Internal,
                    "Local experiment writer thread panicked",
                ));
            }
        }

        self.status
            .ended(completion)
            .write(&self.root.join(STATUS_FILE))
            .map_err(|err| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Internal,
                    "Failed to write the local experiment status",
                    err,
                )
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
        self.sender()?.send(LocalWrite::Lines(lines)).map_err(|_| {
            ExperimentError::new(
                ExperimentErrorKind::Internal,
                "Failed to queue local experiment event",
            )
        })
    }

    fn save_artifact(
        &self,
        name: &str,
        _kind: ArtifactKind,
        artifact: Box<BundleFn>,
    ) -> Result<(), ExperimentError> {
        let artifact_root = self.root.join("artifacts").join(name);
        if artifact_root.exists() {
            fs::remove_dir_all(&artifact_root).map_err(|err| {
                ExperimentError::with_source(
                    ExperimentErrorKind::Artifact,
                    "Failed to replace existing local artifact",
                    err,
                )
            })?;
        }

        let mut bundle = FsBundle::create(artifact_root.clone()).map_err(|err| {
            ExperimentError::with_source(
                ExperimentErrorKind::Artifact,
                "Failed to create local artifact bundle",
                err,
            )
        })?;

        let res = artifact(&mut bundle);

        if res.is_err() {
            _ = bundle.delete();
        }

        res
    }

    fn finish(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        self.finish_worker(completion)
    }
}

/// How a run is going, as its `status.json` gives it.
#[derive(Clone, Serialize)]
struct Status {
    name: String,
    num: u64,
    status: StatusKind,
    started_at: String,
    finished_at: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum StatusKind {
    Running,
    Completed,
    Failed,
}

impl Status {
    /// The status of the run `num` of the experiment `name`, started now.
    fn running(name: String, num: u64) -> Self {
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
    fn ended(&self, completion: ExperimentCompletion) -> Self {
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
    fn write(&self, path: &Path) -> io::Result<()> {
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

struct LocalWorker {
    sender: Sender<LocalWrite>,
    join: JoinHandle<Result<(), io::Error>>,
}

enum LocalWrite {
    /// Lines to append to `events.jsonl`, each ending with a newline.
    Lines(String),
    /// Stop once every line sent before is written.
    Finish,
}

/// One line of `events.jsonl`, with the fields the console's experiment API gives the event.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Line {
    Attribute {
        key: String,
        value: Value,
    },
    Arguments {
        value: Value,
    },
    Config {
        name: String,
        value: Value,
    },
    Log {
        timestamp: String,
        level: &'static str,
        message: String,
        metadata: Map<String, Value>,
        activity: Option<u64>,
    },
    Metric {
        name: String,
        epoch: usize,
        iteration: usize,
        value: f64,
        group: String,
        activity: Option<u64>,
    },
    MetricDefinition {
        name: String,
        description: Option<String>,
        unit: Option<String>,
        higher_is_better: bool,
    },
    EpochSummary {
        name: String,
        epoch: usize,
        group: String,
        value: f64,
        activity: Option<u64>,
    },
    Summary {
        name: String,
        value: f64,
        activity: Option<u64>,
    },
    InputUsed {
        experiment: String,
        name: String,
        artifact_id: String,
    },
    ActivityStarted {
        activity: ActivitySpec,
    },
    ActivityUpdated {
        id: u64,
        current: u64,
    },
    ActivityMessage {
        id: u64,
        message: String,
    },
    ActivityFinished {
        id: u64,
        status: ActivityStatus,
        message: Option<String>,
    },
}

/// The lines `event` is recorded as: one, or one per value of a metric event.
fn lines(event: Event) -> Vec<Line> {
    match event {
        Event::Args(value) => vec![Line::Arguments { value }],
        Event::Config { name, value } => vec![Line::Config { name, value }],
        Event::Log { record, activity } => vec![Line::Log {
            timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            level: record.level.as_str(),
            message: record.message,
            metadata: record.attributes,
            activity: activity.map(|id| id.as_u64()),
        }],
        Event::Metrics {
            epoch,
            split,
            iteration,
            items,
            activity,
        } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::Metric {
                name,
                epoch,
                iteration,
                value,
                group: split.clone(),
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::MetricDefinition(MetricSpec {
            name,
            description,
            unit,
            higher_is_better,
        }) => vec![Line::MetricDefinition {
            name,
            description,
            unit,
            higher_is_better,
        }],
        Event::EpochSummary {
            epoch,
            split,
            items,
            activity,
        } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::EpochSummary {
                name,
                epoch,
                group: split.clone(),
                value,
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::Summary { items, activity } => items
            .into_iter()
            .map(|MetricValue { name, value }| Line::Summary {
                name,
                value,
                activity: activity.map(|id| id.as_u64()),
            })
            .collect(),
        Event::ArtifactUsed {
            experiment_id,
            reference,
        } => vec![Line::InputUsed {
            experiment: experiment_id.to_string(),
            name: reference.name,
            artifact_id: reference.id,
        }],
        Event::Activity(event) => vec![match event {
            ActivityEvent::Started { activity } => Line::ActivityStarted { activity },
            ActivityEvent::Updated { id, current } => Line::ActivityUpdated {
                id: id.as_u64(),
                current,
            },
            ActivityEvent::Message { id, message } => Line::ActivityMessage {
                id: id.as_u64(),
                message,
            },
            ActivityEvent::Finished {
                id,
                status,
                message,
            } => Line::ActivityFinished {
                id: id.as_u64(),
                status,
                message,
            },
        }],
    }
}

/// `lines` as JSON Lines: each one JSON object followed by a newline.
fn to_jsonl(lines: impl IntoIterator<Item = Line>) -> serde_json::Result<String> {
    let mut jsonl = String::new();
    for line in lines {
        jsonl.push_str(&serde_json::to_string(&line)?);
        jsonl.push('\n');
    }
    Ok(jsonl)
}

struct LocalExperimentReader {
    root: PathBuf,
}

impl ExperimentArtifactReader for LocalExperimentReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let experiment_root = parse_local_experiment_root(&self.root, &experiment_id)?;
        let artifact_root = experiment_root.join("artifacts").join(name);

        if !artifact_root.is_dir() {
            return Err(ExperimentReaderError::new(format!(
                "Local artifact not found: {}",
                artifact_root.display()
            )));
        }

        let files = collect_bundle_files(&artifact_root, &artifact_root).map_err(|err| {
            ExperimentReaderError::with_source("Failed to inspect local artifact files", err)
        })?;
        let bundle = FsBundle::with_files(artifact_root.clone(), files).map_err(|err| {
            ExperimentReaderError::with_source("Failed to create local artifact bundle", err)
        })?;

        Ok(LoadedArtifact::new(
            ArtifactRef {
                id: artifact_root.to_string_lossy().to_string(),
                name: name.to_string(),
            },
            bundle,
        ))
    }
}

fn parse_local_experiment_root(
    root: &Path,
    experiment_id: &ExperimentId,
) -> Result<PathBuf, ExperimentReaderError> {
    let experiment_dir = experiment_id.as_str();
    if experiment_dir.is_empty() || experiment_dir.contains('/') || experiment_dir.contains('\\') {
        return Err(ExperimentReaderError::new(
            "Invalid local experiment ID format",
        ));
    }

    Ok(root.join(experiment_dir))
}

/// Appends each batch of lines to `events` as it arrives, until told to finish.
fn local_worker(receiver: Receiver<LocalWrite>, mut events: File) -> Result<(), io::Error> {
    while let Ok(message) = receiver.recv() {
        match message {
            LocalWrite::Lines(lines) => events.write_all(lines.as_bytes())?,
            LocalWrite::Finish => break,
        }
    }
    Ok(())
}

fn collect_bundle_files(root: &Path, current: &Path) -> Result<Vec<String>, std::io::Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_bundle_files(root, &path)?);
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .map_err(std::io::Error::other)?
            .to_string_lossy()
            .to_string();
        files.push(rel);
    }
    Ok(files)
}

/// Creates the directory of the next run under `root`, numbered after the highest one there, and
/// returns its number and path.
fn create_local_run_dir(root: &Path) -> Result<(u64, PathBuf), std::io::Error> {
    let mut next_id = 1u64;

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }

        if let Some(id) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u64>().ok())
        {
            next_id = next_id.max(id + 1);
        }
    }

    loop {
        let run_root = root.join(next_id.to_string());
        match fs::create_dir(&run_root) {
            Ok(()) => return Ok((next_id, run_root)),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                next_id += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use serde_json::json;
    use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

    use super::*;
    use crate::LogRecord;

    /// A one-file artifact.
    #[derive(Debug, PartialEq)]
    struct Note(String);

    impl BundleEncode for Note {
        type Settings = ();
        type Error = String;

        fn encode<O: BundleSink>(self, sink: &mut O, _settings: &()) -> Result<(), String> {
            sink.put_bytes("note.txt", self.0.as_bytes())
        }
    }

    impl BundleDecode for Note {
        type Settings = ();
        type Error = String;

        fn decode<I: BundleSource>(source: &I, _settings: &()) -> Result<Self, String> {
            let mut text = String::new();
            source
                .open("note.txt")?
                .read_to_string(&mut text)
                .map_err(|error| error.to_string())?;
            Ok(Self(text))
        }
    }

    fn run(dir: &Path, name: &str) -> ExperimentRun {
        LocalExperiments::new(dir)
            .create_experiment(name.to_string(), HashMap::new())
            .unwrap()
    }

    #[test]
    fn runs_are_numbered_from_one_under_the_experiment_name() {
        let dir = tempfile::tempdir().unwrap();

        let first = run(dir.path(), "mnist");
        let second = run(dir.path(), "mnist");
        let other = run(dir.path(), "other");

        assert_eq!(first.id().as_str(), "1");
        assert_eq!(second.id().as_str(), "2");
        assert_eq!(other.id().as_str(), "1");
        assert!(dir.path().join("mnist/1").is_dir());
        assert!(dir.path().join("mnist/2").is_dir());
        assert!(dir.path().join("other/1").is_dir());
    }

    #[test]
    fn numbering_continues_after_the_highest_existing_run() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("mnist/7")).unwrap();
        fs::create_dir_all(dir.path().join("mnist/notes")).unwrap();

        assert_eq!(run(dir.path(), "mnist").id().as_str(), "8");
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn a_run_records_each_event_as_a_line_of_json() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = LocalExperiments::new(dir.path())
            .create_experiment(
                "mnist".to_string(),
                HashMap::from([
                    ("tracel.job_num".to_string(), json!(12)),
                    ("kind".to_string(), json!("example")),
                ]),
            )
            .unwrap();

        experiment.log_args(&json!({"epochs": 2})).unwrap();
        experiment
            .log_config("optimizer", &json!({"lr": 0.01}))
            .unwrap();
        experiment.log_metric_definition(MetricSpec {
            name: "loss".to_string(),
            description: Some("training loss".to_string()),
            unit: None,
            higher_is_better: false,
        });
        let epoch = experiment.activity("epoch 1").meter(4, "batch").start();
        epoch.log(LogRecord::warn("slow batch").with("batch", 3));
        epoch.log_metric(
            1,
            "train",
            4,
            vec![
                MetricValue {
                    name: "loss".to_string(),
                    value: 0.5,
                },
                MetricValue {
                    name: "accuracy".to_string(),
                    value: 0.9,
                },
            ],
        );
        epoch.inc(1);
        epoch.finish_with_message("done");
        experiment.log_epoch_summary(
            1,
            "train",
            vec![MetricValue {
                name: "loss".to_string(),
                value: 0.4,
            }],
        );
        experiment.log_summary(vec![MetricValue {
            name: "accuracy".to_string(),
            value: 0.95,
        }]);
        experiment.finish().unwrap();

        let mut lines = read_lines(&dir.path().join("mnist/1/events.jsonl"));
        let timestamp = lines[6]["timestamp"].take();
        assert!(
            timestamp
                .as_str()
                .is_some_and(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok()),
            "{timestamp}"
        );
        assert_eq!(
            lines,
            [
                json!({"type": "attribute", "key": "kind", "value": "example"}),
                json!({"type": "attribute", "key": "tracel.job_num", "value": 12}),
                json!({"type": "arguments", "value": {"epochs": 2}}),
                json!({"type": "config", "name": "optimizer", "value": {"lr": 0.01}}),
                json!({
                    "type": "metric_definition",
                    "name": "loss",
                    "description": "training loss",
                    "unit": null,
                    "higher_is_better": false
                }),
                json!({
                    "type": "activity_started",
                    "activity": {
                        "id": 1,
                        "parent": null,
                        "name": "epoch 1",
                        "cancellable": false,
                        "meter": {"unit": "batch", "total": 4},
                        "attributes": {}
                    }
                }),
                json!({
                    "type": "log",
                    "timestamp": null,
                    "level": "warn",
                    "message": "slow batch",
                    "metadata": {"batch": 3},
                    "activity": 1
                }),
                json!({
                    "type": "metric",
                    "name": "loss",
                    "epoch": 1,
                    "iteration": 4,
                    "value": 0.5,
                    "group": "train",
                    "activity": 1
                }),
                json!({
                    "type": "metric",
                    "name": "accuracy",
                    "epoch": 1,
                    "iteration": 4,
                    "value": 0.9,
                    "group": "train",
                    "activity": 1
                }),
                json!({"type": "activity_updated", "id": 1, "current": 1}),
                json!({
                    "type": "activity_finished",
                    "id": 1,
                    "status": "Success",
                    "message": "done"
                }),
                json!({
                    "type": "epoch_summary",
                    "name": "loss",
                    "epoch": 1,
                    "group": "train",
                    "value": 0.4,
                    "activity": null
                }),
                json!({"type": "summary", "name": "accuracy", "value": 0.95, "activity": null}),
            ]
        );
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

    #[test]
    fn a_run_says_how_it_ended_in_its_status() {
        let dir = tempfile::tempdir().unwrap();
        run(dir.path(), "mnist").fail("diverged").unwrap();
        {
            let cancelled = run(dir.path(), "mnist");
            cancelled.cancel().unwrap();
        }

        let failed = read_json(&dir.path().join("mnist/1/status.json"));
        let cancelled = read_json(&dir.path().join("mnist/2/status.json"));
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["error"], "diverged");
        assert_eq!(cancelled["status"], "completed");
        assert_eq!(cancelled["error"], Value::Null);
    }

    #[test]
    fn a_run_is_located_in_its_directory() {
        let dir = tempfile::tempdir().unwrap();

        let experiment = run(dir.path(), "mnist");

        assert_eq!(
            experiment.location(),
            Some(&ExperimentLocation::Dir(
                dir.path().canonicalize().unwrap().join("mnist/1")
            ))
        );
    }

    #[test]
    fn a_saved_artifact_loads_back_from_a_later_run() {
        let dir = tempfile::tempdir().unwrap();
        let first = run(dir.path(), "mnist");
        first
            .save_artifact("note", ArtifactKind::Other, Note("kept".to_string()), &())
            .unwrap();
        let first_id = first.id().clone();
        first.finish().unwrap();

        let second = run(dir.path(), "mnist");
        let note: Note = second.use_artifact(first_id, "note", &()).unwrap();

        assert_eq!(note, Note("kept".to_string()));
    }

    #[test]
    fn an_experiment_id_naming_another_directory_is_refused() {
        let root = Path::new("/runs/mnist");

        for id in ["", "../other", "a/b", "a\\b"] {
            assert!(
                parse_local_experiment_root(root, &ExperimentId::new(id)).is_err(),
                "{id:?}"
            );
        }
        assert_eq!(
            parse_local_experiment_root(root, &ExperimentId::new("3")).unwrap(),
            root.join("3")
        );
    }
}
