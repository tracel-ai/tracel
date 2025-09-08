use super::context::InferenceContext;
use super::error::InferenceError;
use super::init::Init;
use super::job::JobHandle;
use super::streaming::{CancelToken, CollectEmitter, SyncChannelEmitter};
use crate::inference::model::ModelHost;
use crate::input::RoutineInput;
use crate::output::InferenceOutput;
use crate::routine::ExecutorRoutineWrapper;
use crate::{IntoRoutine, Routine};
use burn::prelude::Backend;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

type ArcInferenceHandler<B, M, I, O, S> =
    Arc<dyn Routine<InferenceContext<B, M, O, S>, In = I, Out = ()>>;

pub struct Inference<B: Backend, M, I, O, S = ()> {
    pub id: String,
    model: ModelHost<M>,
    handler: ArcInferenceHandler<B, M, I, O, S>,
}

impl<B, M, I, O, S> Inference<B, M, I, O, S>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    pub(crate) fn new(id: String, handler: ArcInferenceHandler<B, M, I, O, S>, model: M) -> Self {
        Self {
            id,
            model: ModelHost::spawn(model),
            handler,
        }
    }

    pub fn infer(
        &self,
        input: I::Inner<'static>,
    ) -> super::builder::StrappedInferenceJobBuilder<B, M, I, O, S, super::builder::StateMissing>
    {
        super::builder::StrappedInferenceJobBuilder {
            inference: self,
            input: super::builder::InferenceJobBuilder::new(input),
        }
    }

    pub fn run(
        &self,
        job: super::builder::InferenceJob<B, I, S>,
    ) -> Result<Vec<O>, InferenceError> {
        let collector = Arc::new(CollectEmitter::new());
        let input = job.input;
        let devices = job.devices;
        let state = job.state;
        {
            let mut ctx = InferenceContext {
                id: self.id.clone(),
                devices: devices.into_iter().collect(),
                model: self.model.accessor(),
                emitter: collector.clone(),
                cancel_token: CancelToken::new(),
                state: Mutex::new(Some(state)),
            };
            self.handler
                .run(input, &mut ctx)
                .map_err(|e| InferenceError::HandlerExecutionFailed(e.into()))?;
        }
        let stream = Arc::try_unwrap(collector)
            .map_err(|_| InferenceError::Unexpected("Failed to unwrap collector".to_string()))?
            .into_inner();
        Ok(stream)
    }

    pub fn spawn(&self, job: super::builder::InferenceJob<B, I, S>) -> JobHandle<O>
    where
        <I as RoutineInput>::Inner<'static>: Send,
    {
        let id = self.id.clone();
        let input = job.input;
        let devices = job.devices;
        let state = job.state;
        let (stream_tx, stream_rx) = crossbeam::channel::unbounded();
        let cancel_token = CancelToken::new();
        let mut ctx = InferenceContext {
            id: id.clone(),
            devices: devices.into_iter().collect(),
            model: self.model.accessor(),
            emitter: Arc::new(SyncChannelEmitter::new(stream_tx)),
            cancel_token: cancel_token.clone(),
            state: Mutex::new(Some(state)),
        };
        let handler = self.handler.clone();
        let join = std::thread::spawn(move || {
            handler
                .run(input, &mut ctx)
                .map_err(|e| InferenceError::HandlerExecutionFailed(e.into()))
        });
        JobHandle::new(id, stream_rx, cancel_token, join)
    }

    pub fn into_model(self) -> M {
        self.model.into_model()
    }
}

pub struct InferenceBuilder<B> {
    phantom_data: PhantomData<B>,
}

impl<B: Backend> Default for InferenceBuilder<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<B: Backend> InferenceBuilder<B> {
    pub fn new() -> Self {
        Self {
            phantom_data: Default::default(),
        }
    }

    pub fn init<M, InitArgs>(
        self,
        args: &InitArgs,
        device: &B::Device,
    ) -> Result<LoadedInferenceBuilder<B, M>, M::Error>
    where
        M: Init<B, InitArgs>,
        InitArgs: Send + 'static,
    {
        let model = M::init(args, device)?;
        Ok(LoadedInferenceBuilder {
            model,
            phantom_data: Default::default(),
        })
    }

    pub fn with_model<M>(self, model: M) -> LoadedInferenceBuilder<B, M> {
        LoadedInferenceBuilder {
            model,
            phantom_data: Default::default(),
        }
    }
}

pub struct LoadedInferenceBuilder<B: Backend, M> {
    model: M,
    phantom_data: PhantomData<B>,
}

impl<B, M> LoadedInferenceBuilder<B, M>
where
    B: Backend,
    M: Send + 'static,
{
    pub fn build<F, I, O, RO, Marker, S>(self, handler: F) -> Inference<B, M, I, O, S>
    where
        F: IntoRoutine<InferenceContext<B, M, O, S>, I, RO, Marker>,
        I: RoutineInput + 'static,
        O: Send + 'static,
        S: Send + Sync + 'static,
        RO: InferenceOutput<B, M, O, S> + Sync + 'static,
    {
        Inference::new(
            crate::type_name::fn_type_name::<F>(),
            Arc::new(ExecutorRoutineWrapper::new(IntoRoutine::into_routine(
                handler,
            ))),
            self.model,
        )
    }
}
