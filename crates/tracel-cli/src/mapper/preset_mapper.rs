use std::{collections::HashMap, error::Error};

use crate::mapper::Mapper;

pub struct PresetMapper<I> {
    presets: HashMap<String, I>,
}

impl<I> PresetMapper<I> {
    pub fn new() -> Self {
        Self {
            presets: HashMap::new(),
        }
    }

    pub fn preset(mut self, name: &str, config: I) -> Self {
        self.presets.insert(name.to_string(), config);
        self
    }
}

impl<I: Clone> Mapper<I> for PresetMapper<I> {
    fn map(&self, raw: &str) -> Result<I, Box<dyn Error + Send + Sync>> {
        self.presets.get(raw).cloned().ok_or_else(|| {
            let available: Vec<_> = self.presets.keys().collect();
            format!("unknown preset '{}', available: {:?}", raw, available).into()
        })
    }
}
