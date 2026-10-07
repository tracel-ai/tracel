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
/// Every runner hands a job its input as JSON. The mapper first [resolves](Self::resolve) it into
/// the input the job runs with, which an experiment records as its arguments, then
/// [decodes](Self::decode) that. [`JsonMapper`] merges the input onto a default and decodes it as
/// the input type; [`ClapMapper`] and [`PresetMapper`] take a JSON string as given. The mapper
/// also tells runners what input to expect, through [`example`](Self::example) and
/// [`schema`](Self::schema).
pub trait Mapper<I>: Send + Sync {
    /// Completes `input` into the input the job runs with, such as by merging it onto a
    /// default. `Value::Null` stands for no input.
    ///
    /// Returns `input` as given unless a mapper completes it.
    fn resolve(&self, input: Value) -> Value {
        input
    }

    /// Decodes an input [`resolve`](Self::resolve) returned.
    fn decode(&self, input: Value) -> Result<I, BoxError>;

    /// Resolves `input`, then decodes it.
    fn map(&self, input: Value) -> Result<I, BoxError> {
        self.decode(self.resolve(input))
    }

    /// An example input, listed in the job's definition as `input_example`.
    fn example(&self) -> Option<Value> {
        None
    }

    /// JSON Schema of the input, listed in the job's definition as `input_schema`.
    fn schema(&self) -> Option<Value> {
        None
    }
}
