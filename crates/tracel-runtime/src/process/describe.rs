//! Describing the registered jobs instead of running one.

use std::io;
use std::path::{Path, PathBuf};

use tracel_job::DefinitionsFile;

use crate::JobDefinition;
use crate::process::file::write_atomically;

/// Writes the definitions file of the jobs `definitions` gives to `path`.
pub fn describe<'a>(
    definitions: impl IntoIterator<Item = &'a JobDefinition>,
    path: &Path,
) -> Result<(), DescribeError> {
    let file = DefinitionsFile::new(
        env!("CARGO_PKG_VERSION"),
        definitions.into_iter().cloned().collect(),
    );
    write_atomically(path, &file.to_json()).map_err(|source| DescribeError {
        path: path.to_path_buf(),
        source,
    })
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
    use serde_json::{Value, json};

    use super::*;

    fn definitions() -> Vec<JobDefinition> {
        vec![
            JobDefinition {
                name: "evaluate".to_string(),
                description: None,
                input_schema: None,
                input_example: None,
            },
            JobDefinition {
                name: "train".to_string(),
                description: Some("Train the model".to_string()),
                input_schema: None,
                input_example: Some(json!({"epochs": 10, "optimizer": {"lr": 0.001}})),
            },
        ]
    }

    #[test]
    fn the_definitions_file_lists_every_job() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        describe(&definitions(), &path).unwrap();

        let file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            file,
            json!({
                "protocol": 1,
                "sdk_version": env!("CARGO_PKG_VERSION"),
                "jobs": [
                    {
                        "name": "evaluate",
                        "description": null,
                        "input_schema": null,
                        "input_example": null
                    },
                    {
                        "name": "train",
                        "description": "Train the model",
                        "input_schema": null,
                        "input_example": {"epochs": 10, "optimizer": {"lr": 0.001}}
                    }
                ]
            })
        );
    }

    #[test]
    fn a_failed_write_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("jobs.json");

        let error = describe(&definitions(), &path).unwrap_err();

        assert_eq!(error.path, path);
    }
}
