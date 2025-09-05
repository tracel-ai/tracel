use super::emitter::{CancelToken, Emitter, OutStream};
use super::provider::ModelProvider;
use crate::model::ModelAccessor;
use crate::param::RoutineParam;
use crate::{MultiDevice, State};
use burn::prelude::Backend;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

pub struct InferenceContext<B: Backend, M, O, S> {
    pub id: String,
    pub devices: Vec<B::Device>,
    pub model: ModelAccessor<M>,
    pub emitter: Arc<dyn Emitter<O>>,
    pub cancel_token: Arc<AtomicBool>,
    pub state: Mutex<Option<S>>,
}

// Implementations for extracting parameters from InferenceContext
impl<B: Backend, M, O, S> RoutineParam<InferenceContext<B, M, O, S>> for CancelToken {
    type Item<'new>
        = CancelToken
    where
        B: 'new,
        M: 'new,
        O: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, O, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(CancelToken(ctx.cancel_token.clone()))
    }
}

impl<B: Backend, M, O, S> RoutineParam<InferenceContext<B, M, O, S>> for OutStream<O> {
    type Item<'new>
        = OutStream<O>
    where
        B: 'new,
        M: 'new,
        O: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, O, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(OutStream::new(ctx.emitter.clone()))
    }
}

impl<B: Backend, M: ModelProvider<B>, O, S> RoutineParam<InferenceContext<B, M, O, S>>
    for ModelAccessor<M>
{
    type Item<'new>
        = ModelAccessor<M>
    where
        B: 'new,
        M: 'new,
        O: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, O, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(ctx.model.clone())
    }
}

impl<B: Backend, M, O, S> RoutineParam<InferenceContext<B, M, O, S>> for MultiDevice<B> {
    type Item<'new>
        = MultiDevice<B>
    where
        B: 'new,
        M: 'new,
        O: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, O, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(MultiDevice(ctx.devices.clone()))
    }
}

impl<B: Backend, M, O, S> RoutineParam<InferenceContext<B, M, O, S>> for State<S> {
    type Item<'new>
        = State<S>
    where
        B: 'new,
        M: 'new,
        O: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, O, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(State(ctx.state.lock().unwrap().take().ok_or_else(
            || anyhow::anyhow!("State has already been taken or was not provided"),
        )?))
    }
}
