use std::sync::Arc;

use crate::OutputWriter;
use crate::error::InferenceError;
use crate::inference::{Inference, IntoInference};
use crate::session::InferenceSession;
use crate::stream::InferenceStream;

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

    /// Build a named [`InferenceJob`] from either a type implementing [`Inference`](crate::Inference)
    /// or a closure `Fn(InferenceInput<I>, InferenceOutput<O>)`.
    pub fn create<T, I, O, Marker>(&self, name: &str, inference: T) -> InferenceJob<I, O>
    where
        T: IntoInference<I, O, Marker>,
    {
        let inference: Arc<dyn Inference<Input = I, Output = O> + Send + Sync> =
            Arc::new(inference.into_inference());
        InferenceJob::new(self.provider.clone(), name.to_string(), inference)
    }
}

/// A named inference bound to a backend provider.
///
/// Run it inline on the calling thread with [`run`](Self::run), or spawn a worker and pull outputs
/// back as an iterator with [`stream`](Self::stream) / [`stream_once`](Self::stream_once). Each call
/// opens a fresh per-request [`InferenceSession`] for telemetry.
pub struct InferenceJob<I, O> {
    provider: Arc<dyn InferenceProvider>,
    name: String,
    inference: Arc<dyn Inference<Input = I, Output = O> + Send + Sync>,
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
        inference: Arc<dyn Inference<Input = I, Output = O> + Send + Sync>,
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
    /// Run the inference inline on the calling thread, blocking until it completes.
    ///
    /// Opens a fresh session from the provider and drives the inference under it via
    /// [`InferenceSession::run`]. The returned error covers only a failure to open the session.
    pub fn run<It, W>(&self, input: It, output: W) -> Result<(), InferenceError>
    where
        It: IntoIterator<Item = I>,
        It::IntoIter: Send + 'static,
        W: OutputWriter<O> + 'static,
    {
        let session = self.provider.create_session(&self.name)?;
        session.run(self.inference.as_ref(), input, output);
        Ok(())
    }

    /// Run the inference on a spawned worker, returning its outputs as a pull-based iterator.
    ///
    /// Convenience over [`run`](Self::run) for callers that want to consume outputs directly rather
    /// than supply their own writer. Dropping the returned [`InferenceStream`] cancels the request
    /// and joins the worker.
    pub fn stream<It>(&self, input: It) -> Result<InferenceStream<O>, InferenceError>
    where
        It: IntoIterator<Item = I>,
        It::IntoIter: Send + 'static,
    {
        let session = self.provider.create_session(&self.name)?;
        let inference = self.inference.clone();
        let input = input.into_iter();
        Ok(InferenceStream::spawn(move |channel| {
            session.run(inference.as_ref(), input, channel);
        }))
    }

    /// Run the inference against a single input on a spawned worker.
    pub fn stream_once(&self, input: I) -> Result<InferenceStream<O>, InferenceError> {
        self.stream(std::iter::once(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Inference, InferenceInput, InferenceOutput, InferenceSession};

    struct TestProvider;
    impl InferenceProvider for TestProvider {
        fn create_session(&self, _name: &str) -> Result<InferenceSession, InferenceError> {
            unimplemented!()
        }
    }

    struct Echo;
    impl Inference for Echo {
        type Input = i32;
        type Output = i32;
        fn infer(
            &self,
            _session: &InferenceSession,
            input: InferenceInput<i32>,
            output: InferenceOutput<i32>,
        ) {
            for item in input {
                let _ = output.write(item);
            }
        }
    }

    #[test]
    fn create_accepts_both_impls_and_closures() {
        let module = InferenceModule::new(Arc::new(TestProvider));

        let _from_impl = module.create("impl", Echo);
        let _from_closure = module.create(
            "closure",
            |_session: &InferenceSession,
             input: InferenceInput<i32>,
             output: InferenceOutput<i32>| {
                for item in input {
                    let _ = output.write(item);
                }
            },
        );
    }
}
