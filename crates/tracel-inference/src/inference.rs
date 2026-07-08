use std::marker::PhantomData;
use std::sync::Arc;

use crate::{InferenceInput, InferenceReaderChannel, InferenceWriter, InferenceWriterChannel};

/// A typed, streaming inference task.
///
/// An implementation reads typed inputs from [`InferenceInput`] and writes typed outputs to
/// [`InferenceWriter`]. Both sides stream: the input yields items until the stream ends, and the
/// writer can emit any number of outputs. Any state the task needs (a loaded model, tokenizer, ...)
/// is owned by the implementor and accessed through `&self`, so a single inference can serve many
/// concurrent requests.
///
/// A closure can be adapted into an inference with [`inference_fn`], for cases that don't need a
/// named type.
pub trait Inference: Send + Sync {
    /// The type of each input item pulled from the input stream.
    type Input;
    /// The type of each output item written to the output stream.
    type Output;

    /// Run the inference, pulling inputs and writing outputs until complete.
    fn infer(&self, input: InferenceInput<Self::Input>, writer: InferenceWriter<Self::Output>);
}

/// Adapts a closure `Fn(InferenceInput<I>, InferenceWriter<O>)` into an [`Inference`].
///
/// Build one with [`inference_fn`].
pub struct InferenceFn<F, I, O> {
    f: F,
    _types: PhantomData<fn(I, O)>,
}

/// Wrap a closure into an [`Inference`] implementation.
///
/// ```ignore
/// let echo = inference_fn(|input: InferenceInput<String>, writer: InferenceWriter<String>| {
///     for item in input {
///         let _ = writer.write(item);
///     }
/// });
/// ```
pub fn inference_fn<F, I, O>(f: F) -> InferenceFn<F, I, O>
where
    F: Fn(InferenceInput<I>, InferenceWriter<O>) + Send + Sync,
{
    InferenceFn {
        f,
        _types: PhantomData,
    }
}

impl<F, I, O> Inference for InferenceFn<F, I, O>
where
    F: Fn(InferenceInput<I>, InferenceWriter<O>) + Send + Sync,
{
    type Input = I;
    type Output = O;

    fn infer(&self, input: InferenceInput<I>, writer: InferenceWriter<O>) {
        (self.f)(input, writer)
    }
}

/// A cloneable, `Arc`-backed handle to an [`Inference`] implementation.
pub struct InferenceWrapper<I, O> {
    inner: Arc<dyn Inference<Input = I, Output = O> + Send + Sync>,
}

impl<I, O> Clone for InferenceWrapper<I, O> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<I, O> InferenceWrapper<I, O> {
    /// Wrap an [`Inference`] implementation into a cloneable handle.
    pub fn new<T>(inference: T) -> Self
    where
        T: Inference<Input = I, Output = O> + Send + Sync + 'static,
    {
        Self {
            inner: Arc::new(inference),
        }
    }

    /// Run the inference against raw input and output channels.
    ///
    /// The channels are wrapped into a fresh [`InferenceInput`] / [`InferenceWriter`] with no
    /// observer attached. Use [`infer_prepared`](Self::infer_prepared) when the writer needs an
    /// observer (e.g. session telemetry).
    pub fn infer<RC, WC>(&self, reader: RC, writer: WC)
    where
        RC: InferenceReaderChannel<I> + 'static,
        WC: InferenceWriterChannel<O> + 'static,
    {
        self.inner.infer(
            InferenceInput::from_channel(reader),
            InferenceWriter::from_channel(writer),
        );
    }

    /// Run the inference against a pre-built input and writer.
    pub(crate) fn infer_prepared(&self, input: InferenceInput<I>, writer: InferenceWriter<O>) {
        self.inner.infer(input, writer);
    }
}

impl<I, O> Inference for InferenceWrapper<I, O> {
    type Input = I;
    type Output = O;

    fn infer(&self, input: InferenceInput<I>, writer: InferenceWriter<O>) {
        self.inner.infer(input, writer);
    }
}
