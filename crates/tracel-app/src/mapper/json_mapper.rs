use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::BoxError;
use crate::mapper::Mapper;

/// Decodes a job's input as JSON, merged onto a default when it has one.
///
/// With a default, no input resolves to the default, and an input is merged onto it with JSON
/// merge patch (RFC 7386): its fields replace the default's, and a `null` field removes one. The
/// default is also the job's example input.
pub struct JsonMapper<I> {
    default: Option<Value>,
    schema: Option<Value>,
    _input: PhantomData<fn() -> I>,
}

impl<I> JsonMapper<I> {
    /// Decodes the input as JSON, with no default.
    pub fn new() -> Self {
        Self {
            default: None,
            schema: None,
            _input: PhantomData,
        }
    }

    /// Decodes the input merged onto `default`, which is also the job's example input.
    ///
    /// # Panics
    ///
    /// When `default` does not serialize to JSON.
    pub fn with_default(default: I) -> Self
    where
        I: Serialize,
    {
        Self {
            default: Some(
                serde_json::to_value(default).expect("default input must serialize to JSON"),
            ),
            ..Self::new()
        }
    }

    /// Lists the JSON Schema of `I` as the job's input schema (requires the `schema` feature).
    #[cfg(feature = "schema")]
    pub fn with_schema(mut self) -> Self
    where
        I: schemars::JsonSchema,
    {
        self.schema = Some(schemars::schema_for!(I).to_value());
        self
    }
}

impl<I> Default for JsonMapper<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> Mapper<I> for JsonMapper<I>
where
    I: DeserializeOwned,
{
    fn resolve(&self, input: Value) -> Value {
        match &self.default {
            Some(default) if input.is_null() => default.clone(),
            Some(default) => {
                let mut merged = default.clone();
                json_patch::merge(&mut merged, &input);
                merged
            }
            None => input,
        }
    }

    fn decode(&self, input: Value) -> Result<I, BoxError> {
        Ok(serde_json::from_value(input)?)
    }

    fn example(&self) -> Option<Value> {
        self.default.clone()
    }

    fn schema(&self) -> Option<Value> {
        self.schema.clone()
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Config {
        epochs: u32,
        optimizer: Optimizer,
        tag: Option<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
    struct Optimizer {
        lr: f64,
        weight_decay: f64,
    }

    fn default_config() -> Config {
        Config {
            epochs: 10,
            optimizer: Optimizer {
                lr: 0.001,
                weight_decay: 0.0,
            },
            tag: Some("baseline".to_string()),
        }
    }

    #[test]
    fn without_a_default_the_input_decodes_as_is() {
        let mapper = JsonMapper::<Optimizer>::new();

        let optimizer = mapper
            .map(json!({"lr": 0.1, "weight_decay": 0.01}))
            .unwrap();

        assert_eq!(
            optimizer,
            Optimizer {
                lr: 0.1,
                weight_decay: 0.01
            }
        );
        assert!(mapper.map(Value::Null).is_err());
        assert_eq!(mapper.example(), None);
    }

    #[test]
    fn no_input_yields_the_default() {
        let mapper = JsonMapper::with_default(default_config());

        assert_eq!(mapper.map(Value::Null).unwrap(), default_config());
    }

    #[test]
    fn an_input_is_merge_patched_onto_the_default() {
        let mapper = JsonMapper::with_default(default_config());

        let config = mapper
            .map(json!({"epochs": 2, "optimizer": {"lr": 0.1}, "tag": null}))
            .unwrap();

        assert_eq!(
            config,
            Config {
                epochs: 2,
                optimizer: Optimizer {
                    lr: 0.1,
                    weight_decay: 0.0,
                },
                tag: None,
            }
        );
    }

    #[test]
    fn the_resolved_input_is_the_merged_json() {
        let mapper = JsonMapper::with_default(default_config());

        assert_eq!(
            mapper.resolve(json!({"epochs": 2, "optimizer": {"lr": 0.1}})),
            json!({
                "epochs": 2,
                "optimizer": {"lr": 0.1, "weight_decay": 0.0},
                "tag": "baseline"
            })
        );
        assert_eq!(
            mapper.resolve(Value::Null),
            serde_json::to_value(default_config()).unwrap()
        );
        assert_eq!(
            JsonMapper::<Config>::new().resolve(json!({"epochs": 2})),
            json!({"epochs": 2})
        );
    }

    #[test]
    fn the_default_is_the_example_input() {
        let mapper = JsonMapper::with_default(default_config());

        assert_eq!(
            mapper.example(),
            Some(json!({
                "epochs": 10,
                "optimizer": {"lr": 0.001, "weight_decay": 0.0},
                "tag": "baseline"
            }))
        );
    }

    #[test]
    fn an_input_of_the_wrong_type_does_not_decode() {
        let mapper = JsonMapper::with_default(default_config());

        assert!(mapper.map(json!({"epochs": "ten"})).is_err());
    }

    #[test]
    fn a_mapper_lists_no_schema_unless_asked() {
        assert_eq!(JsonMapper::with_default(default_config()).schema(), None);
    }

    #[cfg(feature = "schema")]
    #[test]
    fn with_schema_lists_the_input_types_schema() {
        let schema = JsonMapper::with_default(default_config())
            .with_schema()
            .schema()
            .unwrap();

        assert_eq!(schema["title"], "Config");
        assert!(schema["properties"]["epochs"].is_object());
        assert!(schema["properties"]["optimizer"].is_object());
    }
}
