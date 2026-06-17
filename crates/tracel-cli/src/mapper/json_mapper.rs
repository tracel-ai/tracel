use std::{error::Error, marker::PhantomData};

use serde::de::DeserializeOwned;

use crate::mapper::Mapper;

pub struct JsonMapper<I> {
    _marker: PhantomData<I>,
}

impl<I> JsonMapper<I> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

impl<I: DeserializeOwned> Mapper<I> for JsonMapper<I> {
    fn map(&self, raw: &str) -> Result<I, Box<dyn Error + Send + Sync>> {
        serde_json::from_str(raw).map_err(Into::into)
    }
}
