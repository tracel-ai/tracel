use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::error::BoxError;

/// Decodes a dispatched job's JSON input into a typed config.
///
/// This is the runner's counterpart to the CLI's `Mapper` (which decodes a `&str`) and the
/// server's `BodyMapper` (which decodes a `&[u8]`); jobs arrive from the station as JSON values.
/// Implement it to support a custom decoding; [`JsonInput`] is provided.
pub trait InputMapper<I>: Send + Sync {
    fn map(&self, input: &Value) -> Result<I, BoxError>;

    /// Example input advertised to the station in the job manifest (`input_example`).
    fn example(&self) -> Option<Value> {
        None
    }
}

/// A JSON input mapper.
///
/// With a default, a null/empty input yields the default and a partial input is merged onto it —
/// the same semantics as `cli::mapper::JsonMapper::with_default` and `server::JsonBody`. The
/// default is also advertised to the station as the job's `input_example`.
pub struct JsonInput<I> {
    default: Option<Value>,
    _marker: PhantomData<fn() -> I>,
}

impl<I> JsonInput<I> {
    /// Decode the input as JSON directly.
    pub fn new() -> Self {
        Self {
            default: None,
            _marker: PhantomData,
        }
    }

    /// Decode the input as JSON merged onto `default` (null/empty input ⇒ the default).
    pub fn with_default(default: I) -> Self
    where
        I: Serialize,
    {
        let default =
            serde_json::to_value(default).expect("default config must be serializable to JSON");
        Self {
            default: Some(default),
            _marker: PhantomData,
        }
    }
}

impl<I> Default for JsonInput<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> InputMapper<I> for JsonInput<I>
where
    I: DeserializeOwned,
{
    fn map(&self, input: &Value) -> Result<I, BoxError> {
        match &self.default {
            Some(default) => {
                if input.is_null() {
                    return Ok(serde_json::from_value(default.clone())?);
                }
                let mut merged = default.clone();
                json_patch::merge(&mut merged, input);
                Ok(serde_json::from_value(merged)?)
            }
            None => Ok(serde_json::from_value(input.clone())?),
        }
    }

    fn example(&self) -> Option<Value> {
        self.default.clone()
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Config {
        epochs: u32,
        lr: f64,
    }

    fn default_config() -> Config {
        Config {
            epochs: 10,
            lr: 0.01,
        }
    }

    #[test]
    fn given_no_default_when_mapping_then_decodes_directly() {
        let mapper = JsonInput::<Config>::new();

        let config = mapper.map(&json!({"epochs": 2, "lr": 0.1})).unwrap();

        assert_eq!(config, Config { epochs: 2, lr: 0.1 });
    }

    #[test]
    fn given_default_when_mapping_partial_input_then_merges_onto_default() {
        let mapper = JsonInput::with_default(default_config());

        let config = mapper.map(&json!({"epochs": 2})).unwrap();

        assert_eq!(
            config,
            Config {
                epochs: 2,
                lr: 0.01
            }
        );
    }

    #[test]
    fn given_default_when_mapping_null_then_yields_default() {
        let mapper = JsonInput::with_default(default_config());

        let config = mapper.map(&Value::Null).unwrap();

        assert_eq!(config, default_config());
    }

    #[test]
    fn given_default_when_asked_for_example_then_returns_it() {
        let mapper = JsonInput::with_default(default_config());

        assert_eq!(mapper.example(), Some(json!({"epochs": 10, "lr": 0.01})));
        assert_eq!(JsonInput::<Config>::new().example(), None);
    }

    #[test]
    fn given_invalid_input_when_mapping_then_errors() {
        let mapper = JsonInput::<Config>::new();

        assert!(mapper.map(&json!({"epochs": "not a number"})).is_err());
    }
}
