use std::error::Error;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub type BoxError = Box<dyn Error + Send + Sync>;

/// Decodes a request body — or, for streaming routes, one framed message — into a typed input.
///
/// This is the server's counterpart to the CLI's [`Mapper`](crate::cli::mapper::Mapper) (which
/// decodes a `&str`). Implement it to support a body format; [`JsonBody`] is provided.
pub trait BodyMapper<I>: Send + Sync {
    fn map(&self, body: &[u8]) -> Result<I, BoxError>;
}

/// A JSON body mapper.
///
/// With a default, an empty body yields the default and a partial body is merged onto it — the
/// server counterpart of [`cli::mapper::JsonMapper::with_default`](crate::cli::mapper::JsonMapper).
pub struct JsonBody<I> {
    default: Option<Value>,
    _marker: PhantomData<fn() -> I>,
}

impl<I> JsonBody<I> {
    /// Decode the body as JSON directly.
    pub fn new() -> Self {
        Self {
            default: None,
            _marker: PhantomData,
        }
    }

    /// Decode the body as JSON merged onto `default` (empty body ⇒ the default).
    pub fn with_default(default: I) -> Self
    where
        I: Serialize,
    {
        let default =
            serde_json::to_value(default).expect("default config must be serializable to JSON");
        Self {
            default: Some(default),
            _marker: PhantomData,
        }
    }
}

impl<I> Default for JsonBody<I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I> BodyMapper<I> for JsonBody<I>
where
    I: DeserializeOwned,
{
    fn map(&self, body: &[u8]) -> Result<I, BoxError> {
        match &self.default {
            Some(default) => {
                if body.trim_ascii().is_empty() {
                    return Ok(serde_json::from_value(default.clone())?);
                }
                let overrides: Value = serde_json::from_slice(body)?;
                let mut merged = default.clone();
                json_patch::merge(&mut merged, &overrides);
                Ok(serde_json::from_value(merged)?)
            }
            None => Ok(serde_json::from_slice(body)?),
        }
    }
}
