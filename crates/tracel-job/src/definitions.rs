//! The definitions file: the jobs a program can run.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PROTOCOL, json_file};

/// Names the file a program writes its [`DefinitionsFile`] to, instead of running a job.
pub const TRACEL_DESCRIBE: &str = "TRACEL_DESCRIBE";

/// Something a runner can run: its name and description, and the input it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDefinition {
    /// The name that selects the job.
    pub name: String,
    /// What the job does.
    pub description: Option<String>,
    /// JSON Schema of the job's input, when its mapper provides one.
    pub input_schema: Option<Value>,
    /// An example input, typically the job's default configuration.
    pub input_example: Option<Value>,
}

/// The file a runner writes to the path [`TRACEL_DESCRIBE`] names: the jobs it can run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionsFile {
    /// The [`PROTOCOL`] version the file follows.
    pub protocol: u32,
    /// The version of the Tracel SDK the program is built with.
    pub sdk_version: String,
    /// The runner that wrote the file, such as `cli` or `server`.
    pub runner: String,
    /// The jobs the runner can run, ordered by name.
    pub jobs: Vec<JobDefinition>,
}

impl DefinitionsFile {
    /// The file of the runner `runner`, such as `cli`, that can run `jobs`: following
    /// [`PROTOCOL`], and built with this version of the SDK.
    pub fn new(runner: impl Into<String>, jobs: Vec<JobDefinition>) -> Self {
        Self {
            protocol: PROTOCOL,
            sdk_version: env!("CARGO_PKG_VERSION").to_string(),
            runner: runner.into(),
            jobs,
        }
    }

    /// Writes the file to `path` atomically: to `<path>.tmp` first, then renamed to `path`.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        json_file::write(path, self)
    }

    /// Reads the file at `path`.
    ///
    /// Fails when it cannot be read or is not a definitions file. It is read whatever its
    /// `protocol`, which the caller compares with [`PROTOCOL`].
    pub fn read(path: &Path) -> io::Result<Self> {
        json_file::read(path)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file() -> DefinitionsFile {
        DefinitionsFile::new(
            "cli",
            vec![
                JobDefinition {
                    name: "echo".to_string(),
                    description: None,
                    input_schema: Some(json!({"type": "string"})),
                    input_example: None,
                },
                JobDefinition {
                    name: "train".to_string(),
                    description: Some("Train the model".to_string()),
                    input_schema: None,
                    input_example: Some(json!({"epochs": 10, "optimizer": {"lr": 0.001}})),
                },
            ],
        )
    }

    #[test]
    fn the_file_is_pretty_printed_json_ending_with_a_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        file().write(&path).unwrap();

        let expected = r#"{
  "protocol": 1,
  "sdk_version": "SDK_VERSION",
  "runner": "cli",
  "jobs": [
    {
      "name": "echo",
      "description": null,
      "input_schema": {
        "type": "string"
      },
      "input_example": null
    },
    {
      "name": "train",
      "description": "Train the model",
      "input_schema": null,
      "input_example": {
        "epochs": 10,
        "optimizer": {
          "lr": 0.001
        }
      }
    }
  ]
}
"#
        .replace("SDK_VERSION", env!("CARGO_PKG_VERSION"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(DefinitionsFile::read(&path).unwrap(), file());
    }

    #[test]
    fn the_file_replaces_an_existing_one_through_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        std::fs::write(&path, "stale").unwrap();

        file().write(&path).unwrap();

        assert_eq!(DefinitionsFile::read(&path).unwrap().jobs.len(), 2);
        assert!(!dir.path().join("jobs.json.tmp").exists());
    }

    #[test]
    fn a_file_that_cannot_be_written_leaves_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("jobs.json");

        assert!(file().write(&path).is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn only_a_definitions_file_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");

        let missing = DefinitionsFile::read(&path).unwrap_err();
        std::fs::write(&path, r#"{"protocol": 1}"#).unwrap();
        let incomplete = DefinitionsFile::read(&path).unwrap_err();

        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        assert_eq!(incomplete.kind(), io::ErrorKind::InvalidData);
    }
}
