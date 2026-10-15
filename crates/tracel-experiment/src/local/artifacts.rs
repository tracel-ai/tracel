use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;
use tracel_artifact::bundle::FsBundle;

use super::runs::child_dir;
use crate::ExperimentId;
use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use crate::session::BundleFn;

/// Saves the artifact `name` of the run recorded in `root`, replacing the one saved before.
///
/// The artifact is written into a temporary directory, which takes the place of the one saved
/// before only once it is complete, so a failed encode leaves that one as it was.
pub fn save_artifact(
    root: &Path,
    name: &str,
    artifact: Box<BundleFn>,
) -> Result<(), ExperimentError> {
    let artifact_root = artifact_dir(root, name)
        .map_err(|message| ExperimentError::new(ExperimentErrorKind::Artifact, message))?;
    let mut staged = FsBundle::temp_in(root.join("artifacts"), ".saving-").map_err(|err| {
        ExperimentError::with_source(
            ExperimentErrorKind::Artifact,
            "Failed to create local artifact bundle",
            err,
        )
    })?;

    artifact(&mut staged)?;

    replace_dir(staged.root(), &artifact_root).map_err(|err| {
        ExperimentError::with_source(
            ExperimentErrorKind::Artifact,
            "Failed to replace existing local artifact",
            err,
        )
    })
}

pub struct LocalExperimentReader {
    pub root: PathBuf,
}

impl ExperimentArtifactReader for LocalExperimentReader {
    fn load_artifact_raw(
        &self,
        experiment_id: ExperimentId,
        name: &str,
    ) -> Result<LoadedArtifact, ExperimentReaderError> {
        let experiment_root = parse_local_experiment_root(&self.root, &experiment_id)?;
        let artifact_root =
            artifact_dir(&experiment_root, name).map_err(ExperimentReaderError::new)?;

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
    child_dir(root, experiment_id.as_str())
        .ok_or_else(|| ExperimentReaderError::new("Invalid local experiment ID format"))
}

/// The directory the artifact `name` of the run recorded in `root` is saved in.
///
/// `name` is a relative path inside the run's `artifacts` directory, such as `checkpoints/best`,
/// a leading `/` included. Fails when it resolves to that directory itself or to one outside it,
/// as `..` does, or when it holds a `\`, which separates paths on Windows only.
fn artifact_dir(root: &Path, name: &str) -> Result<PathBuf, String> {
    let path = RelativePath::new(name).normalize();
    if name.contains('\\') || path.as_str().is_empty() || path.starts_with("..") {
        return Err(format!("Invalid local artifact name: {name:?}"));
    }
    Ok(path.to_path(root.join("artifacts")))
}

/// Moves the directory `staged` to `target`, in place of the one there.
///
/// A directory cannot be renamed over one that is not empty, so the one there is moved aside
/// first, put back if `staged` cannot take its place, and removed once it has.
fn replace_dir(staged: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut replaced = staged.as_os_str().to_owned();
    replaced.push(".replaced");
    let replacing = match fs::rename(target, &replaced) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if let Err(error) = fs::rename(staged, target) {
        if replacing {
            let _ = fs::rename(&replaced, target);
        }
        return Err(error);
    }
    if replacing {
        let _ = fs::remove_dir_all(&replaced);
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Read;

    use rstest::rstest;
    use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

    use super::*;
    use crate::local::test_support::{EXPERIMENT, LocalRun, local_run, run};
    use crate::{ArtifactKind, ExperimentRun};

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

    #[derive(Debug, PartialEq)]
    struct Files(BTreeMap<String, String>);

    impl Files {
        fn of(files: &[(&str, &str)]) -> Self {
            Self(
                files
                    .iter()
                    .map(|(path, contents)| (path.to_string(), contents.to_string()))
                    .collect(),
            )
        }
    }

    impl BundleEncode for Files {
        type Settings = ();
        type Error = String;

        fn encode<O: BundleSink>(self, sink: &mut O, _settings: &()) -> Result<(), String> {
            for (path, contents) in self.0 {
                sink.put_bytes(&path, contents.as_bytes())?;
            }
            Ok(())
        }
    }

    impl BundleDecode for Files {
        type Settings = ();
        type Error = String;

        fn decode<I: BundleSource>(source: &I, _settings: &()) -> Result<Self, String> {
            let mut files = BTreeMap::new();
            for path in source.list()? {
                let mut contents = String::new();
                source
                    .open(&path)?
                    .read_to_string(&mut contents)
                    .map_err(|error| error.to_string())?;
                files.insert(path, contents);
            }
            Ok(Self(files))
        }
    }

    struct HalfWritten;

    impl BundleEncode for HalfWritten {
        type Settings = ();
        type Error = String;

        fn encode<O: BundleSink>(self, sink: &mut O, _settings: &()) -> Result<(), String> {
            sink.put_bytes("note.txt", b"partial")?;
            Err("disk full".to_string())
        }
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
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

    #[rstest]
    #[case::empty("")]
    #[case::current(".")]
    #[case::parent("..")]
    #[case::parent_through_a_nested_name("checkpoints/../..")]
    #[case::windows_separator("..\\outside")]
    fn an_artifact_name_not_inside_the_run_artifacts_is_refused(#[case] name: &str) {
        assert!(artifact_dir(Path::new("run"), name).is_err());
    }

    #[rstest]
    fn an_absolute_artifact_name_is_saved_inside_its_run(local_run: LocalRun) {
        let outside = local_run.runs.path().join("outside");

        local_run
            .run
            .save_artifact(
                outside.to_str().unwrap(),
                ArtifactKind::Other,
                Note("saved".to_string()),
                &(),
            )
            .unwrap();

        assert!(!outside.exists());
    }

    #[rstest]
    fn an_experiment_id_reaching_through_its_parent_is_not_loaded(local_run: LocalRun) {
        local_run
            .run
            .save_artifact("note", ArtifactKind::Other, Note("kept".to_string()), &())
            .unwrap();
        let through_parent = format!("../{EXPERIMENT}/{}", local_run.run.id());

        let loaded = local_run
            .run
            .use_artifact::<Note>(through_parent, "note", &());

        assert!(loaded.is_err());
    }

    fn save_checkpoint(run: &ExperimentRun, files: &[(&str, &str)]) {
        run.save_artifact("checkpoint", ArtifactKind::Model, Files::of(files), &())
            .unwrap();
    }

    #[rstest]
    fn saving_an_artifact_again_replaces_every_file_of_the_one_saved_before(local_run: LocalRun) {
        save_checkpoint(
            &local_run.run,
            &[("model.bin", "v1"), ("optimizer.bin", "v1")],
        );

        save_checkpoint(&local_run.run, &[("model.bin", "v2")]);

        assert_eq!(
            local_run
                .run
                .use_artifact::<Files>(local_run.run.id().clone(), "checkpoint", &())
                .unwrap(),
            Files::of(&[("model.bin", "v2")])
        );
    }

    #[rstest]
    fn replacing_an_artifact_leaves_nothing_else_in_the_run_artifacts(local_run: LocalRun) {
        save_checkpoint(&local_run.run, &[("model.bin", "v1")]);

        save_checkpoint(&local_run.run, &[("model.bin", "v2")]);

        assert_eq!(entries(&local_run.dir.join("artifacts")), ["checkpoint"]);
    }

    #[rstest]
    fn an_artifact_that_fails_to_encode_leaves_the_one_saved_before(local_run: LocalRun) {
        let kept = || Note("kept".to_string());
        local_run
            .run
            .save_artifact("note", ArtifactKind::Other, kept(), &())
            .unwrap();

        local_run
            .run
            .save_artifact("note", ArtifactKind::Other, HalfWritten, &())
            .unwrap_err();

        assert_eq!(
            local_run
                .run
                .use_artifact::<Note>(local_run.run.id().clone(), "note", &())
                .unwrap(),
            kept()
        );
    }

    #[test]
    fn an_artifact_with_a_nested_name_loads_back_with_its_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        let experiment = run(dir.path(), EXPERIMENT);
        let files = || Files::of(&[("config.json", "{}"), ("weights/layer1.bin", "w1")]);

        experiment
            .save_artifact("checkpoints/best", ArtifactKind::Model, files(), &())
            .unwrap();

        assert_eq!(
            experiment
                .use_artifact::<Files>(experiment.id().clone(), "checkpoints/best", &())
                .unwrap(),
            files()
        );
    }
}
