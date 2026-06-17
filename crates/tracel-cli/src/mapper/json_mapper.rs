use std::error::Error;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::mapper::Mapper;

pub struct JsonMapper<I> {
    default: Option<Value>,
    _marker: std::marker::PhantomData<I>,
}

impl<I> JsonMapper<I> {
    pub fn new() -> Self {
        Self {
            default: None,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_default(default: I) -> Self
    where
        I: Serialize,
    {
        Self {
            default: Some(
                serde_json::to_value(default).expect("default config must be serializable"),
            ),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<I: DeserializeOwned> Mapper<I> for JsonMapper<I> {
    fn map(&self, raw: &str) -> Result<I, Box<dyn Error + Send + Sync>> {
        match &self.default {
            Some(default) => {
                let overrides: Value = serde_json::from_str(raw)?;
                let mut merged = default.clone();
                json_patch::merge(&mut merged, &overrides);
                serde_json::from_value(merged).map_err(Into::into)
            }
            None => serde_json::from_str(raw).map_err(Into::into),
        }
    }
}
