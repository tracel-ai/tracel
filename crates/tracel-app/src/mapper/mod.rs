//! Mappers that decode a job's JSON input into the type its job takes.

mod clap_mapper;
mod json_mapper;
mod preset_mapper;

pub use clap_mapper::ClapMapper;
pub use json_mapper::JsonMapper;
pub use preset_mapper::PresetMapper;

use serde_json::Value;

use crate::BoxError;

/// Decodes a job's JSON input into the type its job takes.
///
/// Every runner hands a job its input as JSON. [`JsonMapper`] decodes it as the input type;
/// [`ClapMapper`] and [`PresetMapper`] take a JSON string. The mapper also tells runners what
/// input to expect, through [`example`](Self::example) and [`schema`](Self::schema).
pub trait Mapper<I>: Send + Sync {
    /// Decodes `input`. `Value::Null` stands for no input.
    fn map(&self, input: &Value) -> Result<I, BoxError>;

    /// An example input, listed in the job's definition as `input_example`.
    fn example(&self) -> Option<Value> {
        None
    }

    /// JSON Schema of the input, listed in the job's definition as `input_schema`.
    fn schema(&self) -> Option<Value> {
        None
    }
}
