//! The JSON form of the files: the exact text a program writes, and how a launcher reads it.

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::PROTOCOL;

/// `value` as a file holds it: pretty-printed JSON ending with a newline.
pub fn to_json<T: Serialize>(value: &T) -> String {
    // Every field is a string, a number, a list or JSON, with string keys, which always
    // serializes.
    let mut json = serde_json::to_string_pretty(value).expect("the file serializes to JSON");
    json.push('\n');
    json
}

/// The file `json` holds, when it follows [`PROTOCOL`].
///
/// Its `protocol` is read first, so a file of another protocol fails as such, whatever its other
/// fields.
pub fn from_json<T: DeserializeOwned>(json: &[u8]) -> Result<T, ParseError> {
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

/// Why a document is not a file of [`PROTOCOL`].
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
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn a_file_is_pretty_printed_json_ending_with_a_newline() {
        assert_eq!(
            to_json(&json!({"protocol": 1, "jobs": []})),
            "{\n  \"jobs\": [],\n  \"protocol\": 1\n}\n"
        );
    }

    #[test]
    fn only_a_document_of_this_protocol_parses() {
        let parse = |json: &str| from_json::<Value>(json.as_bytes());

        assert_eq!(
            parse(r#"{"protocol": 1, "job": "train"}"#).unwrap(),
            json!({"protocol": 1, "job": "train"})
        );
        assert!(matches!(
            parse(r#"{"protocol": 2, "jobs": "elsewhere"}"#),
            Err(ParseError::Protocol { found: 2 })
        ));
        assert!(matches!(
            parse(r#"{"jobs": []}"#),
            Err(ParseError::Invalid(_))
        ));
        assert!(matches!(
            parse(r#"{"protocol": 1, "jo"#),
            Err(ParseError::Invalid(_))
        ));
    }

    #[test]
    fn a_protocol_error_names_both_versions() {
        let error = from_json::<Value>(br#"{"protocol": 7}"#).unwrap_err();

        assert_eq!(error.to_string(), "protocol 7 is not supported, expected 1");
    }
}
