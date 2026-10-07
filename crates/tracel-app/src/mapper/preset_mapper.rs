use std::collections::BTreeMap;

use serde_json::Value;

use crate::BoxError;
use crate::mapper::Mapper;

/// Decodes a job's input as the name of a preset input.
///
/// The input is a JSON string naming one of the presets, such as `"small"`. The job lists no
/// example input or schema.
pub struct PresetMapper<I> {
    presets: BTreeMap<String, I>,
}

impl<I> PresetMapper<I> {
    /// A mapper with no presets yet.
    pub fn new() -> Self {
        Self {
            presets: BTreeMap::new(),
        }
    }

    /// Adds `input` as the preset `name`.
    pub fn preset(mut self, name: &str, input: I) -> Self {
        self.presets.insert(name.to_string(), input);
        self
    }

    fn names(&self) -> String {
        let names: Vec<&str> = self.presets.keys().map(String::as_str).collect();
        names.join(", ")
    }
}

impl<I> Default for PresetMapper<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> Mapper<I> for PresetMapper<I>
where
    I: Clone + Send + Sync,
{
    fn map(&self, input: &Value) -> Result<I, BoxError> {
        let Some(name) = input.as_str() else {
            return Err(format!(
                "expected a JSON string naming a preset, one of: {}",
                self.names()
            )
            .into());
        };
        self.presets
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown preset '{name}', available: {}", self.names()).into())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_json_string_selects_a_preset() {
        let mapper = PresetMapper::new().preset("small", 1).preset("large", 8);

        assert_eq!(mapper.map(&json!("large")).unwrap(), 8);
    }

    #[test]
    fn an_unknown_or_missing_preset_lists_the_available_ones() {
        let mapper = PresetMapper::new().preset("small", 1).preset("large", 8);

        let unknown = mapper.map(&json!("medium")).unwrap_err().to_string();
        let missing = mapper.map(&Value::Null).unwrap_err().to_string();

        assert!(unknown.contains("large, small"), "{unknown}");
        assert!(missing.contains("large, small"), "{missing}");
        assert_eq!(mapper.example(), None);
        assert_eq!(mapper.schema(), None);
    }
}
