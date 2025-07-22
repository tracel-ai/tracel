use json_patch::merge;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    PatchError(json_patch::PatchError),
    #[error(transparent)]
    Serialization(serde_json::Error),
}

impl Error {
    pub fn is_syntax(&self) -> bool {
        matches!(self, Error::Serialization(e) if e.is_syntax())
    }

    pub fn is_data(&self) -> bool {
        matches!(self, Error::Serialization(e) if e.is_data())
    }
}

pub trait ExperimentConfig: Serialize + for<'de> Deserialize<'de> + Default {}
impl<T> ExperimentConfig for T where T: Serialize + for<'de> Deserialize<'de> + Default {}

pub fn deserialize_and_merge_with_default<T: ExperimentConfig>(config: &str) -> Result<T, Error> {
    let override_val: serde_json::Value =
        serde_json::from_str(config).map_err(Error::Serialization)?;

    let mut merged = serde_json::to_value(T::default()).map_err(Error::Serialization)?;

    merge(&mut merged, &override_val);

    serde_json::from_value(merged).map_err(Error::Serialization)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    /// A nested struct to test deep merges.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Nested {
        x: bool,
        y: u64,
    }

    impl Default for Nested {
        fn default() -> Self {
            Nested { x: true, y: 10 }
        }
    }

    /// A config type covering primitives, Option, nested structs, and arrays.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct MyConfig {
        a: i32,
        b: Option<String>,
        nested: Nested,
        list: Vec<i32>,
    }

    impl Default for MyConfig {
        fn default() -> Self {
            MyConfig {
                a: 5,
                b: Some("hello".to_owned()),
                nested: Nested::default(),
                list: vec![1, 2, 3],
            }
        }
    }

    #[test]
    fn empty_override_returns_default() {
        let cfg: MyConfig = deserialize_and_merge_with_default("{}").unwrap();
        assert_eq!(cfg, MyConfig::default());
    }

    #[test]
    fn override_top_level_field() {
        let cfg: MyConfig = deserialize_and_merge_with_default(r#"{ "a": 42 }"#).unwrap();
        let mut expected = MyConfig::default();
        expected.a = 42;
        assert_eq!(cfg, expected);
    }

    #[test]
    fn deep_override_nested_field() {
        let cfg: MyConfig =
            deserialize_and_merge_with_default(r#"{ "nested": { "y": 99 } }"#).unwrap();
        let mut expected = MyConfig::default();
        expected.nested.y = 99;
        // nested.x stays default = true
        assert_eq!(cfg, expected);
    }

    #[test]
    fn null_becomes_json_null_for_optional() {
        let cfg: MyConfig = deserialize_and_merge_with_default(r#"{ "b": null }"#).unwrap();
        // Option<String> → None
        assert_eq!(cfg.b, None);
    }

    #[test]
    fn null_becomes_json_null_for_required() {
        // Setting a non-Option field to null yields literal null in JSON,
        // which then causes a data error on deserialization.
        let err = deserialize_and_merge_with_default::<MyConfig>(r#"{ "a": null }"#).unwrap_err();
        assert!(err.is_data());
    }

    #[test]
    fn override_list_replaces_array() {
        let cfg: MyConfig = deserialize_and_merge_with_default(r#"{ "list": [9,8,7] }"#).unwrap();
        assert_eq!(cfg.list, vec![9, 8, 7]);
    }

    #[test]
    fn invalid_json_input_errors_syntax() {
        let err = deserialize_and_merge_with_default::<MyConfig>("not json").unwrap_err();
        assert!(err.is_syntax());
    }

    #[test]
    fn type_mismatch_in_nested_errors_data() {
        // Trying to apply a boolean where an object is expected
        let err =
            deserialize_and_merge_with_default::<MyConfig>(r#"{ "nested": false }"#).unwrap_err();
        assert!(err.is_data());
    }

    #[test]
    fn patch_application_error_propagates() {
        // Create a patch that tries to remove a non-existent path
        // by feeding a value that isn’t even an object at the root.
        // For example, override is an array → diff/patch will still succeed,
        // but final from_value will error for shape mismatch.
        let err = deserialize_and_merge_with_default::<MyConfig>(r#"[1,2,3]"#).unwrap_err();
        // Should be a data error because root must be an object
        assert!(err.is_data());
    }
}
