pub struct ExperimentConfig {
    pub data: serde_json::Value,
}

impl ExperimentConfig {
    fn new(value: serde_json::Value) -> Self {
        Self { data: value }
    }

    fn apply_override(&mut self, key_path: &str, value: serde_json::Value) {
        let mut parts = key_path.split('.').peekable();
        let mut target = &mut self.data;

        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                // Last part, set value
                if let serde_json::Value::Object(map) = target {
                    map.insert(part.to_string(), value.clone());
                }
            } else {
                target = target
                    .as_object_mut()
                    .unwrap()
                    .entry(part)
                    .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            }
        }
    }

    pub fn load_config(path: Option<String>, overrides: Vec<(String, serde_json::Value)>) -> Self {
        let base_json = if let Some(path) = &path {
            let text = std::fs::read_to_string(path).expect("failed to read config file");
            serde_json::from_str(&text).expect("failed to parse config file")
        } else {
            serde_json::json!({})
        };

        let mut config = ExperimentConfig::new(base_json);

        for (key, val) in &overrides {
            config.apply_override(key, val.clone());
        }

        config
    }
}
