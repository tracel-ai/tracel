use std::sync::Arc;

use tracel_task::{Job, Streaming};

use crate::OutputWriter;
use crate::error::InferenceError;
use crate::inference::{Inference, IntoInference};
use crate::session::InferenceSession;
use crate::stream::channel;

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Backend port that creates per-request [`InferenceSession`]s.
///
/// Implementations decide how a request's telemetry is observed and shipped. The session is
/// handed back as a [`Job`] any executor can drive: an implementation whose transport needs a
/// runtime attaches the work to the one it owns.
pub trait InferenceProvider: Send + Sync + 'static {
    /// Create a session for one request of the inference named `name`.
    fn create_session(&self, name: String) -> Job<InferenceSession, InferenceError>;
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
/// [`run`](Self::run) hands back the request as a [`Job`] whose outputs go to a writer of the
/// caller's; [`stream`](Self::stream) / [`stream_once`](Self::stream_once) pair that job with a
/// [`Streaming`] of its outputs. The inference itself is synchronous compute and runs wherever
/// the job is driven. Each call opens a fresh per-request [`InferenceSession`] for telemetry.
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
    /// The request as a job: opens a session from the provider, then drives the inference under
    /// it via [`InferenceSession::run`], writing outputs to `output`.
    ///
    /// The inference is synchronous compute and runs inline wherever the job is driven; a caller
    /// that must not stall its executor for the duration hands the job to a blocking thread. The
    /// job's error covers only a failure to open the session.
    pub fn run<It, W>(&self, input: It, output: W) -> Job<(), InferenceError>
    where
        It: IntoIterator<Item = I>,
        It::IntoIter: Send + 'static,
        W: OutputWriter<O> + Send + 'static,
    {
        let session = self.provider.create_session(self.name.clone());
        let inference = self.inference.clone();
        let input = input.into_iter();
        Job::new(async move {
            session.await?.run(inference.as_ref(), input, output);
            Ok(())
        })
    }

    /// The request as a job paired with a stream of its outputs.
    ///
    /// Drive the job wherever the compute should run and pull the outputs from anywhere; the
    /// channel between them is unbounded, so neither waits on the other. Dropping the stream
    /// cancels the request.
    pub fn stream<It>(&self, input: It) -> (Job<(), InferenceError>, Streaming<O, BoxError>)
    where
        It: IntoIterator<Item = I>,
        It::IntoIter: Send + 'static,
    {
        let (writer, outputs) = channel();
        (self.run(input, writer), outputs)
    }

    /// [`stream`](Self::stream) for a single input.
    pub fn stream_once(&self, input: I) -> (Job<(), InferenceError>, Streaming<O, BoxError>) {
        self.stream(std::iter::once(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Inference, InferenceInput, InferenceOutput, InferenceSession};

    struct TestProvider;
    impl InferenceProvider for TestProvider {
        fn create_session(&self, _name: String) -> Job<InferenceSession, InferenceError> {
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
