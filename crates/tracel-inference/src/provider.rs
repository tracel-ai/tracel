use std::sync::Arc;

use crate::error::InferenceError;
use crate::inference::{InferenceFn, inference_fn};
use crate::session::InferenceSession;
use crate::stream::{DirectInference, InferenceStream};
use crate::writer::InferenceWriter;
use crate::{Inference, InferenceInput, InferenceWrapper};

/// Backend port that creates per-request [`InferenceSession`]s.
///
/// Implementations decide how a request's telemetry is observed and shipped.
pub trait InferenceProvider: Send + Sync + 'static {
    /// Create a session for one request of the inference named `name`.
    fn create_session(&self, name: &str) -> Result<InferenceSession, InferenceError>;
}

/// Entry point for building inference jobs against a backend.
#[derive(Clone)]
pub struct InferenceModule {
    provider: Arc<dyn InferenceProvider>,
}

impl InferenceModule {
    /// Create a module backed by the given provider.
    pub fn new(provider: Arc<dyn InferenceProvider>) -> Self {
        Self { provider }
    }

    /// Wrap an inference implementation into a named [`InferenceJob`].
    ///
    /// `inference` may be a type implementing [`Inference`] (owning its own state) or a closure
    /// `Fn(InferenceInput<I>, InferenceWriter<O>)`.
    pub fn create<T>(&self, name: &str, inference: T) -> InferenceJob<T::Input, T::Output>
    where
        T: Inference + Send + Sync + 'static,
    {
        InferenceJob::new(
            self.provider.clone(),
            name.to_string(),
            InferenceWrapper::new(inference),
        )
    }

    /// Wrap an inference closure into a named [`InferenceJob`].
    ///
    /// Convenience over [`create`](Self::create) + [`inference_fn`].
    pub fn create_fn<F, I, O>(&self, name: &str, f: F) -> InferenceJob<I, O>
    where
        F: Fn(InferenceInput<I>, InferenceWriter<O>) + Send + Sync + 'static,
        I: 'static,
        O: 'static,
    {
        let inference: InferenceFn<F, I, O> = inference_fn(f);
        self.create(name, inference)
    }
}

/// A named inference bound to a backend provider.
///
/// Run it with [`stream`](Self::stream) / [`stream_once`](Self::stream_once); each call opens a
/// fresh per-request session and returns a typed [`InferenceStream`] of outputs.
pub struct InferenceJob<I, O> {
    provider: Arc<dyn InferenceProvider>,
    name: String,
    inference: InferenceWrapper<I, O>,
}

impl<I, O> Clone for InferenceJob<I, O> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            name: self.name.clone(),
            inference: self.inference.clone(),
        }
    }
}

impl<I, O> InferenceJob<I, O> {
    fn new(
        provider: Arc<dyn InferenceProvider>,
        name: String,
        inference: InferenceWrapper<I, O>,
    ) -> Self {
        Self {
            provider,
            name,
            inference,
        }
    }

    /// The job's name, used to select it from a CLI or HTTP request path.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl<I, O> InferenceJob<I, O>
where
    I: Send + 'static,
    O: Send + Sync + 'static,
{
    /// Run the inference against a stream of inputs, opening a fresh session for the request.
    pub fn stream<It>(&self, input: It) -> Result<InferenceStream<O>, InferenceError>
    where
        It: IntoIterator<Item = I>,
        It::IntoIter: Send + 'static,
    {
        let session = self.provider.create_session(&self.name)?;
        let direct = DirectInference::new(self.inference.clone());
        Ok(direct.stream_with_session(input, Some(session)))
    }

    /// Run the inference against a single input, opening a fresh session for the request.
    pub fn stream_once(&self, input: I) -> Result<InferenceStream<O>, InferenceError> {
        self.stream(std::iter::once(input))
    }
}
