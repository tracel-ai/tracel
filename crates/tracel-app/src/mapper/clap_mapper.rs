use std::marker::PhantomData;

use clap::Parser;
use serde_json::Value;

use crate::BoxError;
use crate::mapper::Mapper;

/// Decodes a job's input as command-line arguments for a [`clap::Parser`] type.
///
/// The input is a JSON string of whitespace-separated arguments, such as `"--epochs 3"`; no input
/// parses no arguments. The job lists no example input or schema.
pub struct ClapMapper<I> {
    _input: PhantomData<fn() -> I>,
}

impl<I> ClapMapper<I> {
    /// Parses the input as `I`'s command-line arguments.
    pub fn new() -> Self {
        Self {
            _input: PhantomData,
        }
    }
}

impl<I> Default for ClapMapper<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Parser> Mapper<I> for ClapMapper<I> {
    fn map(&self, input: &Value) -> Result<I, BoxError> {
        let arguments = match input {
            Value::Null => "",
            Value::String(arguments) => arguments,
            other => {
                return Err(format!(
                    "expected a JSON string of command-line arguments, got {other}"
                )
                .into());
            }
        };
        let arguments = std::iter::once("").chain(arguments.split_whitespace());
        Ok(I::try_parse_from(arguments)?)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[derive(Debug, PartialEq, Parser)]
    struct Args {
        #[arg(long, default_value_t = 1)]
        epochs: u32,
    }

    #[test]
    fn a_json_string_parses_as_arguments() {
        let mapper = ClapMapper::<Args>::new();

        assert_eq!(
            mapper.map(&json!("--epochs 3")).unwrap(),
            Args { epochs: 3 }
        );
        assert_eq!(mapper.map(&Value::Null).unwrap(), Args { epochs: 1 });
    }

    #[test]
    fn an_input_that_is_not_a_string_does_not_decode() {
        let mapper = ClapMapper::<Args>::new();

        assert!(mapper.map(&json!({"epochs": 3})).is_err());
        assert!(mapper.map(&json!("--unknown")).is_err());
        assert_eq!(mapper.example(), None);
        assert_eq!(mapper.schema(), None);
    }
}
