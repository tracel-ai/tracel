//! The definitions file: the jobs a program can run.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PROTOCOL;
use crate::json::{self, ParseError};

/// Names the file a program writes its [`DefinitionsFile`] to, instead of running a job.
pub const TRACEL_DESCRIBE: &str = "TRACEL_DESCRIBE";

/// A job a program can run: its name and description, and the input it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDefinition {
    /// The name that selects the job.
    pub name: String,
    /// What the job does.
    pub description: Option<String>,
    /// JSON Schema of the job's input, when the program gives one.
    pub input_schema: Option<Value>,
    /// An example input, typically the job's default input, which its input is merged onto.
    pub input_example: Option<Value>,
}

/// The file a program writes to the path [`TRACEL_DESCRIBE`] names: the jobs it can run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DefinitionsFile {
    /// The [`PROTOCOL`] version the file follows.
    pub protocol: u32,
    /// The version of the Tracel SDK the program is built with.
    pub sdk_version: String,
    /// The jobs the program can run, ordered by name.
    pub jobs: Vec<JobDefinition>,
}

impl DefinitionsFile {
    /// The file of a program that can run `jobs`: following [`PROTOCOL`], and built with this
    /// version of the SDK.
    pub fn new(jobs: Vec<JobDefinition>) -> Self {
        Self {
            protocol: PROTOCOL,
            sdk_version: env!("CARGO_PKG_VERSION").to_string(),
            jobs,
        }
    }

    /// The file's contents: pretty-printed JSON ending with a newline.
    pub fn to_json(&self) -> String {
        json::to_json(self)
    }

    /// The file `json` holds.
    ///
    /// Fails with [`ParseError::Protocol`] when it follows another [`PROTOCOL`] version, and with
    /// [`ParseError::Invalid`] when it is not a definitions file.
    pub fn from_json(json: &[u8]) -> Result<Self, ParseError> {
        json::from_json(json)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn file() -> DefinitionsFile {
        DefinitionsFile::new(vec![
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
        ])
    }

    #[test]
    fn the_file_is_pretty_printed_json_ending_with_a_newline() {
        let expected = r#"{
  "protocol": 1,
  "sdk_version": "SDK_VERSION",
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

        assert_eq!(file().to_json(), expected);
        assert_eq!(
            DefinitionsFile::from_json(expected.as_bytes()).unwrap(),
            file()
        );
    }

    #[test]
    fn only_a_definitions_file_of_this_protocol_parses() {
        let incomplete = DefinitionsFile::from_json(br#"{"protocol": 1}"#);
        let other = DefinitionsFile::from_json(br#"{"protocol": 2, "jobs": {}}"#);

        assert!(matches!(incomplete, Err(ParseError::Invalid(_))));
        assert!(matches!(other, Err(ParseError::Protocol { found: 2 })));
    }
}
