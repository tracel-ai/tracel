use std::fs;
use std::path::{Path, PathBuf};

use tracel_artifact::bundle::FsBundle;

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
    let artifact_root = root.join("artifacts").join(name);
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

    use tracel_artifact::bundle::{BundleDecode, BundleEncode, BundleSink, BundleSource};

    use super::*;
    use crate::ArtifactKind;
    use crate::local::test_support::run;

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
