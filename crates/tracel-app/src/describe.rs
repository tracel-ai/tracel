//! Describing the registered jobs instead of running one.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use tracel_job::{DefinitionsFile, TRACEL_DESCRIBE};

use crate::registry::JobRegistry;

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
        let path = PathBuf::from(path);
        DefinitionsFile::new(runner, self.definitions().cloned().collect())
            .write(&path)
            .map_err(|source| DescribeError { path, source })?;
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
    use std::path::Path;
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
        assert_eq!(
            DefinitionsFile::read(&path).unwrap().jobs,
            registry().definitions().cloned().collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_failed_write_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("jobs.json");

        let error = describe_to(&path).unwrap_err();

        assert!(error.to_string().contains("jobs.json"), "{error}");
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
