use std::marker::PhantomData;

use crate::{InferenceInput, InferenceOutput};

/// A typed, streaming inference task.
///
/// An implementation reads typed inputs from [`InferenceInput`] and writes typed outputs to
/// [`InferenceOutput`]. Both sides stream: the input yields items until the stream ends, and the
/// output can emit any number of outputs. Any state the task needs (a loaded model, tokenizer, ...)
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
    fn infer(&self, input: InferenceInput<Self::Input>, output: InferenceOutput<Self::Output>);
}

/// Adapts a closure `Fn(InferenceInput<I>, InferenceOutput<O>)` into an [`Inference`].
///
/// Build one with [`inference_fn`].
pub struct InferenceFn<F, I, O> {
    f: F,
    _types: PhantomData<fn(I, O)>,
}

/// Wrap a closure into an [`Inference`] implementation.
///
/// ```ignore
/// let echo = inference_fn(|input: InferenceInput<String>, output: InferenceOutput<String>| {
///     for item in input {
///         let _ = output.write(item);
///     }
/// });
/// ```
pub fn inference_fn<F, I, O>(f: F) -> InferenceFn<F, I, O>
where
    F: Fn(InferenceInput<I>, InferenceOutput<O>) + Send + Sync,
{
    InferenceFn {
        f,
        _types: PhantomData,
    }
}

impl<F, I, O> Inference for InferenceFn<F, I, O>
where
    F: Fn(InferenceInput<I>, InferenceOutput<O>) + Send + Sync,
{
    type Input = I;
    type Output = O;

    fn infer(&self, input: InferenceInput<I>, output: InferenceOutput<O>) {
        (self.f)(input, output)
    }
}

/// Conversion into an [`Inference`], accepting both types that implement [`Inference`] and closures
/// `Fn(InferenceInput<I>, InferenceOutput<O>)`. `Marker` disambiguates the two blanket impls and is
/// inferred at the call site.
pub trait IntoInference<I, O, Marker> {
    type Inference: Inference<Input = I, Output = O> + Send + Sync + 'static;

    fn into_inference(self) -> Self::Inference;
}

/// Marker for the blanket impl over types that already implement [`Inference`].
pub struct IsInference;

impl<T> IntoInference<T::Input, T::Output, IsInference> for T
where
    T: Inference + Send + Sync + 'static,
{
    type Inference = T;

    fn into_inference(self) -> T {
        self
    }
}

/// Marker for the blanket impl over closures.
pub struct IsFn;

impl<F, I, O> IntoInference<I, O, IsFn> for F
where
    F: Fn(InferenceInput<I>, InferenceOutput<O>) + Send + Sync + 'static,
    I: 'static,
    O: 'static,
{
    type Inference = InferenceFn<F, I, O>;

    fn into_inference(self) -> InferenceFn<F, I, O> {
        inference_fn(self)
    }
}
