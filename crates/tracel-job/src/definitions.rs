//! The definitions file: the jobs a program can run.

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PROTOCOL;

/// Names the file a program writes its [`DefinitionsFile`] to, instead of running a job.
pub const TRACEL_DESCRIBE: &str = "TRACEL_DESCRIBE";

/// A job the program can run, as its definitions file gives it: its name and description, and the
/// input it takes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobDefinition {
    /// The name that selects the job.
    pub name: String,
    /// What the job does.
    pub description: Option<String>,
    /// JSON Schema of the job's input, when the program gives one.
    pub input_schema: Option<Value>,
    /// The job's default input, which its input is merged onto.
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
    /// The file of a program built with version `sdk_version` of the Tracel SDK that can run
    /// `jobs`, following [`PROTOCOL`], with `jobs` ordered by name.
    pub fn new(sdk_version: impl Into<String>, mut jobs: Vec<JobDefinition>) -> Self {
        jobs.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            protocol: PROTOCOL,
            sdk_version: sdk_version.into(),
            jobs,
        }
    }

    /// The file's contents: pretty-printed JSON ending with a newline.
    pub fn to_json(&self) -> String {
        // Every field is a string, a number, a list or JSON, with string keys, which always
        // serializes.
        let mut json = serde_json::to_string_pretty(self).expect("the file serializes to JSON");
        json.push('\n');
        json
    }

    /// The file `json` holds.
    ///
    /// Its `protocol` is read first, so a file of another protocol fails as such, whatever its
    /// other fields: with [`ParseError::Protocol`] when it follows another [`PROTOCOL`] version,
    /// and with [`ParseError::Invalid`] when it is not a definitions file.
    pub fn from_json(json: &[u8]) -> Result<Self, ParseError> {
        #[derive(Deserialize)]
        struct Header {
            protocol: u32,
        }

        let Header { protocol } = serde_json::from_slice(json).map_err(ParseError::Invalid)?;
        if protocol != PROTOCOL {
            return Err(ParseError::Protocol { found: protocol });
        }
        serde_json::from_slice(json).map_err(ParseError::Invalid)
    }
}

/// Why a document is not a definitions file of [`PROTOCOL`].
#[derive(Debug)]
pub enum ParseError {
    /// The document is not JSON of the file's form, or is cut short.
    Invalid(serde_json::Error),
    /// The document follows another protocol version.
    Protocol {
        /// The `protocol` the document gives.
        found: u32,
    },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(error) => error.fmt(f),
            Self::Protocol { found } => {
                write!(f, "protocol {found} is not supported, expected {PROTOCOL}")
            }
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(error) => Some(error),
            Self::Protocol { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_jobs_are_ordered_by_name() {
        let job = |name: &str| JobDefinition {
            name: name.to_string(),
            description: None,
            input_schema: None,
            input_example: None,
        };

        let file =
            DefinitionsFile::new("0.10.0", vec![job("train"), job("evaluate"), job("infer")]);

        assert_eq!(file.jobs, [job("evaluate"), job("infer"), job("train")]);
    }

    #[test]
    fn a_file_of_another_protocol_is_refused_whatever_its_other_fields() {
        let other = PROTOCOL + 1;
        let document = json!({"protocol": other, "jobs": "elsewhere"});

        let parsed = DefinitionsFile::from_json(document.to_string().as_bytes());

        assert!(matches!(parsed, Err(ParseError::Protocol { found }) if found == other));
    }
}
