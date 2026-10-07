//! Recording experiments on this machine, without reaching any server.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use serde_json::Value;
use tracel_artifact::bundle::FsBundle;

use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use crate::session::{BundleFn, Event, ExperimentCompletion, ExperimentSession};
use crate::{ArtifactKind, ExperimentId, ExperimentProvider, ExperimentRun};

/// Records experiments under a directory on this machine.
///
/// The runs of the experiment named `name` are numbered from 1 under `<dir>/<name>`. Each run
/// appends its events to `events.log`, writes its completion to `status.txt`, and saves its
/// artifacts under `artifacts/`. Creating a `LocalExperiments` performs no I/O; the first run
/// creates the directories.
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
        _attributes: HashMap<String, Value>,
    ) -> Result<ExperimentRun, ExperimentError> {
        create_experiment_run(self.dir.join(name))
    }
}

fn create_experiment_run(root: PathBuf) -> Result<ExperimentRun, ExperimentError> {
    let root = root
        .canonicalize()
        .or_else(|_| {
            fs::create_dir_all(&root)?;
            root.canonicalize()
        })
        .map_err(|err| {
            ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Failed to create local experiment directory",
                err,
            )
        })?;
    let (id, run_root) = create_local_run_dir(&root).map_err(|err| {
        ExperimentError::with_source(
            ExperimentErrorKind::Internal,
            "Failed to create local experiment run directory",
            err,
        )
    })?;

    let session = LocalExperimentSession::new(run_root).map_err(|err| {
        ExperimentError::with_source(
            ExperimentErrorKind::Internal,
            "Failed to initialize local experiment session",
            err,
        )
    })?;
    let reader = LocalExperimentReader { root };

    Ok(ExperimentRun::new(id, session, reader, Default::default()))
}

struct LocalExperimentSession {
    root: PathBuf,
    active: Mutex<Option<LocalWorker>>,
}

impl LocalExperimentSession {
    fn new(root: PathBuf) -> Result<Self, std::io::Error> {
        fs::create_dir_all(root.join("artifacts"))?;
        let (sender, receiver) = channel();
        let events_path = root.join("events.log");
        let status_path = root.join("status.txt");
        let join = thread::spawn(move || local_worker(receiver, events_path, status_path));

        Ok(Self {
            root,
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

    fn finish_worker(&self, completion: ExperimentCompletion) -> Result<(), ExperimentError> {
        let worker = self.active.lock().unwrap().take().ok_or_else(|| {
            ExperimentError::new(
                ExperimentErrorKind::AlreadyFinished,
                "Local experiment session has already finished",
            )
        })?;

        let send_result = worker
            .sender
            .send(LocalWrite::Finish(format!("{completion:?}")));
        let join_result = worker.join.join();

        match join_result {
            Ok(Ok(())) => {
                if send_result.is_err() {
                    return Err(ExperimentError::new(
                        ExperimentErrorKind::Internal,
                        "Failed to send local experiment completion",
                    ));
                }
                Ok(())
            }
            Ok(Err(err)) => Err(ExperimentError::with_source(
                ExperimentErrorKind::Internal,
                "Local experiment writer failed",
                err,
            )),
            Err(_) => Err(ExperimentError::new(
                ExperimentErrorKind::Internal,
                "Local experiment writer thread panicked",
            )),
        }
    }
}

impl ExperimentSession for LocalExperimentSession {
    fn record_event(&self, event: Event) -> Result<(), ExperimentError> {
        self.sender()?
            .send(LocalWrite::Event(format!("{event:?}")))
            .map_err(|_| {
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

struct LocalWorker {
    sender: Sender<LocalWrite>,
    join: JoinHandle<Result<(), std::io::Error>>,
}

enum LocalWrite {
    Event(String),
    Finish(String),
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

fn append_line(path: &Path, line: &str) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(())
}

fn local_worker(
    receiver: Receiver<LocalWrite>,
    events_path: PathBuf,
    status_path: PathBuf,
) -> Result<(), std::io::Error> {
    while let Ok(message) = receiver.recv() {
        match message {
            LocalWrite::Event(line) => append_line(&events_path, &line)?,
            LocalWrite::Finish(line) => {
                append_line(&status_path, &line)?;
                return Ok(());
            }
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

fn create_local_run_dir(root: &Path) -> Result<(ExperimentId, PathBuf), std::io::Error> {
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
        let id = next_id.to_string();
        let run_root = root.join(&id);
        match fs::create_dir(&run_root) {
            Ok(()) => return Ok((ExperimentId::from(id), run_root)),
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

    use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

    use super::*;

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

    #[test]
    fn a_finished_run_leaves_its_events_and_completion() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = run(dir.path(), "mnist");

        experiment
            .log_args(&serde_json::json!({ "epochs": 2 }))
            .unwrap();
        experiment.finish().unwrap();

        let events = fs::read_to_string(dir.path().join("mnist/1/events.log")).unwrap();
        let status = fs::read_to_string(dir.path().join("mnist/1/status.txt")).unwrap();
        assert!(events.contains("epochs"), "{events}");
        assert_eq!(status.trim(), "Success");
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
