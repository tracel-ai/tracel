use std::fs;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;
use tracel_artifact::bundle::FsBundle;

use super::runs::child_dir;
use crate::ExperimentId;
use crate::error::{ExperimentError, ExperimentErrorKind};
use crate::reader::{ArtifactRef, ExperimentArtifactReader, ExperimentReaderError, LoadedArtifact};
use crate::session::BundleFn;

/// Saves the artifact `name` of the run recorded in `root`, replacing the one saved before.
pub fn save_artifact(
    root: &Path,
    name: &str,
    artifact: Box<BundleFn>,
) -> Result<(), ExperimentError> {
    let artifact_root = artifact_dir(root, name)
        .map_err(|message| ExperimentError::new(ExperimentErrorKind::Artifact, message))?;
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
    use std::io::Read;

    use rstest::rstest;
    use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

    use super::*;
    use crate::ArtifactKind;
    use crate::local::test_support::{EXPERIMENT, LocalRun, local_run, run};

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
}
