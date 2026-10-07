use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::job::JobDefinition;
use crate::registry::JobRegistry;

/// Names the file a runner writes its job definitions to, instead of running a job.
const TRACEL_DESCRIBE: &str = "TRACEL_DESCRIBE";

/// The runner protocol version a definitions file follows.
const PROTOCOL: u32 = 1;

/// The file a runner writes to the path `TRACEL_DESCRIBE` names: the jobs it can run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionsFile {
    /// The runner protocol version, `1`.
    pub protocol: u32,
    /// The version of the Tracel SDK the program is built with.
    pub sdk_version: String,
    /// The runner that wrote the file, such as `cli` or `server`.
    pub runner: String,
    /// The jobs the runner can run, ordered by name.
    pub jobs: Vec<JobDefinition>,
}

impl DefinitionsFile {
    fn new(runner: &str, jobs: Vec<JobDefinition>) -> Self {
        Self {
            protocol: PROTOCOL,
            sdk_version: env!("CARGO_PKG_VERSION").to_string(),
            runner: runner.to_string(),
            jobs,
        }
    }

    /// Writes the file to `path` atomically: to `<path>.tmp` first, then renamed to `path`.
    fn write(&self, path: &Path) -> Result<(), DescribeError> {
        let failed = |source| DescribeError {
            path: path.to_path_buf(),
            source,
        };
        let mut contents = serde_json::to_vec_pretty(self).map_err(|e| failed(e.into()))?;
        contents.push(b'\n');

        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if let Err(source) =
            std::fs::write(&tmp, contents).and_then(|()| std::fs::rename(&tmp, path))
        {
            let _ = std::fs::remove_file(&tmp);
            return Err(failed(source));
        }
        Ok(())
    }
}

impl JobRegistry {
    /// Writes the definitions file for `runner` when `TRACEL_DESCRIBE` names a path.
    ///
    /// Returns whether it did; a runner that did returns without running a job. `runner` names
    /// the runner in the file, such as `cli`.
    pub fn describe_from_env(&self, runner: &str) -> Result<bool, DescribeError> {
        self.describe_with_vars(runner, |name| std::env::var_os(name))
    }

    /// [`describe_from_env`](Self::describe_from_env) with the environment variables `lookup`
    /// gives.
    fn describe_with_vars(
        &self,
        runner: &str,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<bool, DescribeError> {
        let Some(path) = lookup(TRACEL_DESCRIBE).filter(|path| !path.is_empty()) else {
            return Ok(false);
        };
        DefinitionsFile::new(runner, self.definitions().cloned().collect())
            .write(Path::new(&path))?;
        Ok(true)
    }
}

/// The definitions file could not be written.
#[derive(Debug, thiserror::Error)]
#[error("failed to write the job definitions to {}: {source}", path.display())]
pub struct DescribeError {
    path: PathBuf,
    source: io::Error,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tracel_experiment::{ExperimentRun, Experiments};
    use tracel_inference::{
        InferenceInput, InferenceModule, InferenceOutput, InferenceSession, NoopInferenceProvider,
    };

    use super::*;
    use crate::job::IntoJob;
    use crate::mapper::JsonMapper;
    use crate::test_support::NeverRuns;

    fn registry() -> JobRegistry {
        let mut registry = JobRegistry::new();
        registry.add(
            Experiments::new(Arc::new(NeverRuns))
                .create("train", |_run: &ExperimentRun, _input: Value| Ok(()))
                .with_description("Train the model")
                .into_job(JsonMapper::with_default(
                    json!({"epochs": 10, "optimizer": {"lr": 0.001}}),
                )),
        );
        registry.add(
            InferenceModule::new(Arc::new(NoopInferenceProvider::new()))
                .create(
                    "echo",
                    |_session: &InferenceSession,
                     input: InferenceInput<String>,
                     output: InferenceOutput<String>| {
                        for text in input {
                            let _ = output.write(text);
                        }
                    },
                )
                .into_job(JsonMapper::<String>::new()),
        );
        registry
    }

    fn describe_to(path: &Path) -> Result<bool, DescribeError> {
        registry().describe_with_vars("cli", |name| {
            (name == TRACEL_DESCRIBE).then(|| path.as_os_str().to_owned())
        })
    }

    #[test]
    fn the_definitions_file_lists_every_registered_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        assert!(describe_to(&path).unwrap());

        let contents = std::fs::read_to_string(&path).unwrap();
        let file: Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            file,
            json!({
                "protocol": 1,
                "sdk_version": env!("CARGO_PKG_VERSION"),
                "runner": "cli",
                "jobs": [
                    {
                        "name": "echo",
                        "kind": "inference",
                        "description": null,
                        "input_schema": null,
                        "input_example": null
                    },
                    {
                        "name": "train",
                        "kind": "experiment",
                        "description": "Train the model",
                        "input_schema": null,
                        "input_example": {"epochs": 10, "optimizer": {"lr": 0.001}}
                    }
                ]
            })
        );
        assert!(contents.ends_with('\n'));
        let parsed: DefinitionsFile = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            parsed.jobs,
            registry().definitions().cloned().collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_file_replaces_an_existing_one_through_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        std::fs::write(&path, "stale").unwrap();

        describe_to(&path).unwrap();

        let file: DefinitionsFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file.jobs.len(), 2);
        assert!(!dir.path().join("jobs.json.tmp").exists());
    }

    #[test]
    fn a_failed_write_names_the_path_and_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("jobs.json");

        let error = describe_to(&path).unwrap_err();

        assert!(error.to_string().contains("jobs.json"), "{error}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn an_unset_or_empty_tracel_describe_writes_nothing() {
        let registry = registry();

        assert!(!registry.describe_with_vars("cli", |_| None).unwrap());
        assert!(
            !registry
                .describe_with_vars("cli", |_| Some(OsString::new()))
                .unwrap()
        );
    }
}
