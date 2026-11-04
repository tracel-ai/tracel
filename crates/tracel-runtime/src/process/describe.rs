//! Describing the registered jobs instead of running one.

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use tracel_job::{DefinitionsFile, TRACEL_DESCRIBE};

use super::protocol_definition;
use crate::JobDefinition;
use crate::process::file::write_atomically;

/// Writes the definitions file of the jobs `definitions` gives when `TRACEL_DESCRIBE`, among the
/// environment variables `lookup` gives, names a path.
///
/// Returns whether it did; the program then runs no job.
pub fn describe<'a>(
    definitions: impl IntoIterator<Item = &'a JobDefinition>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<bool, DescribeError> {
    let Some(path) = lookup(TRACEL_DESCRIBE).filter(|path| !path.is_empty()) else {
        return Ok(false);
    };
    let path = PathBuf::from(path);
    let file = DefinitionsFile::new(definitions.into_iter().map(protocol_definition).collect());
    write_atomically(&path, &file.to_json()).map_err(|source| DescribeError { path, source })?;
    Ok(true)
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

    fn describe_to(path: &Path) -> Result<bool, DescribeError> {
        describe(&definitions(), |name| {
            (name == TRACEL_DESCRIBE).then(|| path.as_os_str().to_owned())
        })
    }

    #[test]
    fn the_definitions_file_lists_every_job() {
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
        let read = DefinitionsFile::from_json(contents.as_bytes()).unwrap();
        assert_eq!(
            read.jobs,
            definitions()
                .iter()
                .map(protocol_definition)
                .collect::<Vec<_>>()
        );
        assert_eq!(contents, read.to_json());
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
        assert!(!describe(&definitions(), |_| None).unwrap());
        assert!(!describe(&definitions(), |_| Some(OsString::new())).unwrap());
    }
}
