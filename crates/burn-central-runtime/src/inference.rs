use crate::input::RoutineInput;
use crate::model::{ModelAccessor, ModelHost};
use crate::output::InferenceOutput;
use crate::param::RoutineParam;
use crate::routine::ExecutorRoutineWrapper;
use crate::{IntoRoutine, MultiDevice, Routine, State};
use burn::prelude::Backend;
use burn_central_client::BurnCentral;
use burn_central_client::model::{ModelRegistry, ModelRegistryError, ModelSpec};
use derive_more::Deref;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::TrySendError;
use std::sync::{Arc, Mutex};
use thiserror::Error;

mod job {
    use crate::inference::{CancelToken, InferenceError};
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    pub struct JobHandle<S> {
        pub id: String,
        pub stream: mpsc::Receiver<S>,
        cancel: CancelToken,
        join: Option<JoinHandle<Result<(), InferenceError>>>,
    }
    impl<S> JobHandle<S> {
        pub fn new(
            id: String,
            stream: mpsc::Receiver<S>,
            cancel: CancelToken,
            join: JoinHandle<Result<(), InferenceError>>,
        ) -> Self {
            Self {
                id,
                stream,
                cancel,
                join: Some(join),
            }
        }
        pub fn cancel(&self) {
            self.cancel.cancel();
        }
        pub fn join(mut self) -> Result<(), InferenceError> {
            if let Some(join) = self.join.take() {
                Ok(join.join().unwrap_or_else(|e| {
                    Err(InferenceError::ThreadPanicked(format!(
                        "Inference thread panicked: {:?}",
                        e
                    )))
                })?)
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EmitControl {
    Continue,
    Stop,
}

pub trait Emitter<T>: Send + Sync + 'static {
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError>;
    fn end(&self) -> Result<(), InferenceError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub struct CollectEmitter<T>(Mutex<Vec<T>>);
impl<T> CollectEmitter<T> {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
    pub fn into_inner(self) -> Vec<T> {
        self.0.into_inner().unwrap()
    }
}
impl<T: Send + 'static> Emitter<T> for CollectEmitter<T> {
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError> {
        self.0.lock().unwrap().push(item);
        Ok(EmitControl::Continue)
    }
}

pub struct SyncChannelEmitter<T> {
    tx: std::sync::mpsc::SyncSender<T>,
}

impl<T: Send + 'static> SyncChannelEmitter<T> {
    pub fn new(tx: std::sync::mpsc::SyncSender<T>) -> Self {
        Self { tx }
    }
}

impl<T: Send + 'static> Emitter<T> for SyncChannelEmitter<T> {
    fn emit(&self, item: T) -> Result<EmitControl, InferenceError> {
        match self.tx.try_send(item) {
            Ok(_) => Ok(EmitControl::Continue),
            Err(TrySendError::Full(_)) => Ok(EmitControl::Stop),
            Err(TrySendError::Disconnected(_)) => Ok(EmitControl::Stop),
        }
    }
}

#[derive(Clone, Deref)]
pub struct OutStream<T> {
    emitter: Arc<dyn Emitter<T>>,
}

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model loading failed: {0}")]
    ModelLoadingFailed(#[from] ModelProviderError),
    #[error("Inference handler execution failed: {0}")]
    HandlerExecutionFailed(anyhow::Error),
    #[error("Inference cancelled")]
    Cancelled,
    #[error("Unexpected error: {0}")]
    Unexpected(String),
    #[error("Inference thread panicked: {0}")]
    ThreadPanicked(String),
}

pub struct InferenceContext<B: Backend, M, O, S> {
    pub id: String,
    pub devices: Vec<B::Device>,
    pub model: ModelAccessor<M>,
    pub emitter: Arc<dyn Emitter<O>>,
    pub cancel_token: Arc<AtomicBool>,
    pub state: Mutex<Option<S>>,
}

#[derive(Debug, Error)]
pub enum ModelProviderError {
    #[error("Model registry error: {0}")]
    ModelLoadingFailed(#[from] ModelRegistryError),
}
type ModelProviderResult<M> = Result<M, ModelProviderError>;
pub trait ModelProvider<B: Backend>: Sized {
    fn get_model(
        registry: &ModelRegistry,
        model_spec: ModelSpec,
        device: &B::Device,
    ) -> ModelProviderResult<Self>;
}

// cancellation token
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
        Ok(OutStream {
            emitter: ctx.emitter.clone(),
        })
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

type ArcInferenceHandler<B, M, I, O, S> =
    Arc<dyn Routine<InferenceContext<B, M, O, S>, In = I, Out = ()>>;

pub struct Inference<B: Backend, M, I, O, S = ()> {
    pub id: String,
    model: ModelHost<M>,
    handler: ArcInferenceHandler<B, M, I, O, S>,
    _burn_central: BurnCentral,
    phantom_data: PhantomData<(O, S)>,
}

impl<B, M, I, O, S> Inference<B, M, I, O, S>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    pub fn infer(
        &self,
        input: I::Inner<'static>,
    ) -> StrappedInferenceJobBuilder<B, M, I, O, S, StateMissing> {
        StrappedInferenceJobBuilder {
            inference: self,
            input: InferenceJobBuilder::new(input),
        }
    }
}

impl<B, M, I, O, S> Inference<B, M, I, O, S>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    fn new(
        id: String,
        handler: ArcInferenceHandler<B, M, I, O, S>,
        model: M,
        client: BurnCentral,
    ) -> Self {
        Self {
            id,
            model: ModelHost::spawn(model),
            handler,
            _burn_central: client,
            phantom_data: Default::default(),
        }
    }

    pub fn run(&self, job: InferenceJob<B, I, S>) -> Result<Vec<O>, InferenceError> {
        let collector = Arc::new(CollectEmitter::new());
        let input = job.input;
        let devices = job.devices;
        let state = job.state;
        {
            let mut ctx = InferenceContext {
                id: self.id.clone(),
                devices: devices.into_iter().collect(),
                model: self.model.get_accessor(),
                emitter: collector.clone(),
                cancel_token: Arc::new(AtomicBool::new(false)),
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

    pub fn spawn(&self, job: InferenceJob<B, I, S>) -> job::JobHandle<O>
    where
        <I as RoutineInput>::Inner<'static>: Send,
    {
        let id = self.id.clone();
        let input = job.input;
        let devices = job.devices;
        let state = job.state;
        let (stream_tx, stream_rx) = std::sync::mpsc::sync_channel(10);
        let cancel_token = CancelToken::new();
        let mut ctx = InferenceContext {
            id: id.clone(),
            devices: devices.into_iter().collect(),
            model: self.model.get_accessor(),
            emitter: Arc::new(SyncChannelEmitter::new(stream_tx)),
            cancel_token: cancel_token.0.clone(),
            state: Mutex::new(Some(state)),
        };
        let handler = self.handler.clone();
        let join = std::thread::spawn(move || {
            handler
                .run(input, &mut ctx)
                .map_err(|e| InferenceError::HandlerExecutionFailed(e.into()))
        });
        job::JobHandle::new(id, stream_rx, cancel_token, join)
    }
}

pub struct InferenceBuilder<B> {
    client: BurnCentral,
    phantom_data: PhantomData<B>,
}

impl<B: Backend> InferenceBuilder<B> {
    pub fn new(client: BurnCentral) -> Self {
        Self {
            client,
            phantom_data: Default::default(),
        }
    }

    pub fn load<M: ModelProvider<B>>(
        self,
        model_spec: ModelSpec,
        device: &B::Device,
    ) -> Result<LoadedInferenceBuilder<B, M>, InferenceError> {
        let registry = self.client.model_registry();
        let model = M::get_model(&registry, model_spec, device)
            .map_err(InferenceError::ModelLoadingFailed)?;
        Ok(LoadedInferenceBuilder {
            client: self.client,
            model,
            phantom_data: Default::default(),
        })
    }
}
pub struct LoadedInferenceBuilder<B: Backend, M> {
    client: BurnCentral,
    model: M,
    phantom_data: PhantomData<B>,
}

impl<B, M> LoadedInferenceBuilder<B, M>
where
    B: Backend,
    M: Send + 'static,
{
    pub fn build<'a, F, I, O, RO, Marker, S>(self, handler: F) -> Inference<B, M, I, O, S>
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
            self.client,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{In, MultiDevice, Out, State};
    use burn::backend::NdArray;
    use burn::nn::{Linear, LinearConfig};
    use burn::prelude::{Backend, Module};
    use burn::tensor::Tensor;
    use burn_central_client::credentials::BurnCentralCredentials;
    use std::str::FromStr;

    type TestBackend = NdArray;
    type Device = <TestBackend as Backend>::Device;

    #[derive(Module, Debug)]
    pub struct MyNnModel<B: Backend> {
        linear: Linear<B>,
    }

    impl<B: Backend> MyNnModel<B> {
        fn new(device: &B::Device) -> Self {
            let linear = LinearConfig::new(10, 5).init(device);
            MyNnModel { linear }
        }
    }

    impl<B: Backend> ModelProvider<B> for MyNnModel<B> {
        fn get_model(
            registry: &ModelRegistry,
            model_spec: ModelSpec,
            device: &B::Device,
        ) -> ModelProviderResult<Self> {
            println!("Fetching model from registry with spec: {model_spec}");

            let model_artifact = registry.get_model(model_spec)?;

            println!("Model artifact fetched: {:?}", model_artifact);

            // let config = model_artifact.get_config();
            // let weights = model_artifact.get_weights();

            Ok(MyNnModel::new(device))
        }
    }

    fn my_inference_function<B: Backend>(
        In(input): In<Tensor<B, 2>>,
        model: ModelAccessor<MyNnModel<B>>,
        devices: MultiDevice<B>,
        output: OutStream<Tensor<B, 2>>,
    ) -> Result<(), InferenceError> {
        // println!("Running inference with model: {:?}", *model);
        println!("Using devices: {:?}", devices[0]);
        println!("Input tensor: {}", input);
        let mut result = input;
        for i in 0..5 {
            result = model.with(move |m| m.linear.forward(result + i));
            if output.emit(result.clone()).is_err() {
                println!("Emitter requested to stop, exiting inference function.");
                return Err(InferenceError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        Ok(())
    }

    fn my_direct_inference_function<B: Backend>(
        In(input): In<Tensor<B, 2>>,
        model: ModelAccessor<MyNnModel<B>>,
        _devices: MultiDevice<B>,
        state: State<i32>,
    ) -> Out<Tensor<B, 2>> {
        println!("Running direct inference with model: {:?}", model);
        let result = model.with(move |m| m.linear.forward(input));
        result.into()
    }

    fn panicking_inference_function<B: Backend>(
        _input: In<Tensor<B, 2>>,
        _model: ModelAccessor<MyNnModel<B>>,
        _devices: MultiDevice<B>,
        output: OutStream<String>,
        State(state): State<String>,
    ) {
        println!("Initial state: {}", state);
        for i in 0..5 {
            if output.emit(format!("Processing step {}", i)).is_err() {
                println!("Emitter requested to stop, exiting inference function.");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        panic!("This inference function always panics");
    }

    fn provision_client() -> BurnCentral {
        let creds = BurnCentralCredentials::from_str("7052e19b-7d6c-48ed-baad-053d91121f58")
            .expect("Should be able to create credentials");

        let client = BurnCentral::builder(creds)
            .with_endpoint("http://localhost:9001")
            .build()
            .expect("Should be able to login");
        client
    }

    #[test]
    fn test_inference_creation() {
        fn infer<B: Backend>() {
            let client = provision_client();

            let device = Default::default();

            let inference = InferenceBuilder::new(client)
                .load("test/proj/my_nn_model:1".parse().unwrap(), &device)
                .unwrap()
                .build(my_inference_function);

            let input = Tensor::<B, 2>::ones([1, 10], &device);
            println!("{}", serde_json::to_string(&input.to_data()).unwrap());

            let output = inference.infer(input).with_devices([device]).run().unwrap();
            println!("Inference output: {:?}", output);
        }

        infer::<TestBackend>();
    }

    #[test]
    fn test_inference_job_spawn() {
        let client = provision_client();

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<MyNnModel<TestBackend>>("test/proj/my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(my_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        println!("{}", serde_json::to_string(&input).unwrap());
        let job = inference.infer(input).with_devices([device]).spawn();
        for output in job.stream.iter() {
            println!("Received output: {}", output);
            job.cancel();
        }
        let res = job.join();
        if let Err(e) = res {
            println!("Job ended with error: {}", e);
        } else {
            println!("Job completed successfully");
        }
    }

    #[test]
    fn test_panicking_job_spawn() {
        let client = provision_client();

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<MyNnModel<TestBackend>>("test/proj/my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(panicking_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        println!("{}", serde_json::to_string(&input).unwrap());
        let state = String::from("Initial state");
        let job = inference
            .infer(input.clone())
            .with_devices([device])
            .with_state(state.clone())
            .spawn();

        let job = InferenceJob::builder(input)
            .with_devices([device])
            .with_state(state)
            .build();

        let job = inference.spawn(job);

        for output in job.stream.iter() {
            println!("Received output: {}", output);
        }
        let res = job.join();
        if let Err(e) = res {
            println!("Job ended with error: {}", e);
        } else {
            println!("Job completed successfully");
        }
    }

    #[test]
    fn test_inference_with_direct_return() {
        let client = provision_client();

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client)
            .load::<MyNnModel<TestBackend>>("test/proj/my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(my_direct_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        let output = inference
            .infer(input)
            .with_devices([device])
            .with_state(32)
            .run();
        println!("Direct inference output: {:?}", output);
    }
}

pub struct StrappedInferenceJobBuilder<'a, B: Backend, M, I: RoutineInput, O, S, Flag> {
    inference: &'a Inference<B, M, I, O, S>,
    input: InferenceJobBuilder<B, I, S, Flag>,
}

impl<'a, B, M, I, O, S, Flag> StrappedInferenceJobBuilder<'a, B, M, I, O, S, Flag>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    pub fn with_devices(mut self, devices: impl IntoIterator<Item = B::Device>) -> Self {
        self.input = self.input.with_devices(devices);
        self
    }
}

impl<'a, B, M, I, O, S> StrappedInferenceJobBuilder<'a, B, M, I, O, S, StateMissing>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    pub fn with_state(
        self,
        state: S,
    ) -> StrappedInferenceJobBuilder<'a, B, M, I, O, S, StateProvided> {
        StrappedInferenceJobBuilder {
            inference: self.inference,
            input: self.input.with_state(state),
        }
    }
}

pub struct InferenceJobBuilder<B: Backend, I: RoutineInput, S, Flag> {
    input: <I as RoutineInput>::Inner<'static>,
    devices: Vec<B::Device>,
    state: Option<S>,
    _flag: PhantomData<Flag>,
}

impl<B, I, S, Flag> InferenceJobBuilder<B, I, S, Flag>
where
    B: Backend,
    I: RoutineInput + 'static,
    S: Send + Sync + 'static,
{
    pub fn new(input: <I as RoutineInput>::Inner<'static>) -> Self {
        Self {
            input,
            devices: Vec::new(),
            state: None,
            _flag: PhantomData,
        }
    }

    pub fn with_devices(mut self, devices: impl IntoIterator<Item = B::Device>) -> Self {
        self.devices = devices.into_iter().collect();
        self
    }
}

pub struct StateMissing;
pub struct StateProvided;

impl<B, I, S> InferenceJobBuilder<B, I, S, StateMissing>
where
    B: Backend,
    I: RoutineInput + 'static,
    S: Send + Sync + 'static,
{
    pub fn with_state(self, state: S) -> InferenceJobBuilder<B, I, S, StateProvided> {
        InferenceJobBuilder {
            input: self.input,
            devices: self.devices,
            state: Some(state),
            _flag: PhantomData,
        }
    }
}

impl<B, I, S> InferenceJobBuilder<B, I, S, StateProvided>
where
    B: Backend,
    I: RoutineInput + 'static,
    S: Send + Sync + 'static,
{
    pub fn build(self) -> InferenceJob<B, I, S> {
        InferenceJob {
            input: self.input,
            devices: self.devices,
            state: self.state.expect("state must be set"),
        }
    }
}

impl<'a, B, M, I, O> StrappedInferenceJobBuilder<'a, B, M, I, O, (), StateMissing>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
{
    pub fn spawn(self) -> job::JobHandle<O>
    where
        <I as RoutineInput>::Inner<'static>: Send,
    {
        let job = InferenceJob {
            input: self.input.input,
            devices: self.input.devices,
            state: (),
        };
        self.inference.spawn(job)
    }

    pub fn run(self) -> Result<Vec<O>, InferenceError> {
        let job = InferenceJob {
            input: self.input.input,
            devices: self.input.devices,
            state: (),
        };
        self.inference.run(job)
    }
}

impl<'a, B, M, I, O, S> StrappedInferenceJobBuilder<'a, B, M, I, O, S, StateProvided>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
    S: Send + Sync + 'static,
{
    pub fn spawn(self) -> job::JobHandle<O>
    where
        <I as RoutineInput>::Inner<'static>: Send,
    {
        let job = InferenceJob {
            input: self.input.input,
            devices: self.input.devices,
            state: self.input.state.expect("state must be set"),
        };
        self.inference.spawn(job)
    }

    pub fn run(self) -> Result<Vec<O>, InferenceError> {
        let job = InferenceJob {
            input: self.input.input,
            devices: self.input.devices,
            state: self.input.state.expect("state must be set"),
        };
        self.inference.run(job)
    }
}

pub struct InferenceJob<B: Backend, I: RoutineInput, S> {
    input: <I as RoutineInput>::Inner<'static>,
    devices: Vec<B::Device>,
    state: S,
}

impl<B, I, S> InferenceJob<B, I, S>
where
    B: Backend,
    I: RoutineInput + 'static,
    S: Send + Sync + 'static,
{
    pub fn builder(
        input: <I as RoutineInput>::Inner<'static>,
    ) -> InferenceJobBuilder<B, I, S, StateMissing> {
        InferenceJobBuilder::new(input)
    }
}
