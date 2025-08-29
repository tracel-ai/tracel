use crate::input::RoutineInput;
use crate::param::RoutineParam;
use crate::{In, IntoRoutine, Model, MultiDevice, Routine};
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

// cancellation (sync-friendly)
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

// collects into Vec<T> (for blocking calls / tests)
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
pub struct Out<T> {
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

pub struct InferenceContext<B: Backend, M, S> {
    pub id: String,
    pub devices: Vec<B::Device>,
    pub state: Arc<InferenceState<M>>,
    pub emitter: Arc<dyn Emitter<S>>,
    pub cancel_token: Arc<AtomicBool>,
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
impl<B: Backend, M, S> RoutineParam<InferenceContext<B, M, S>> for CancelToken {
    type Item<'new>
        = CancelToken
    where
        B: 'new,
        M: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(CancelToken(ctx.cancel_token.clone()))
    }
}
impl<B: Backend, M, S> RoutineParam<InferenceContext<B, M, S>> for Out<S> {
    type Item<'new>
        = Out<S>
    where
        B: 'new,
        M: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(Out {
            emitter: ctx.emitter.clone(),
        })
    }
}

impl<B: Backend, M: Clone + ModelProvider<B>, S> RoutineParam<InferenceContext<B, M, S>>
    for Model<M>
{
    type Item<'new>
        = Model<M>
    where
        M: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(Model(
            ctx.state
                .model
                .lock()
                .map_err(|e| anyhow::anyhow!("Failed to lock model mutex: {}", e))?
                .clone(),
        ))
    }
}

impl<B: Backend, M, S> RoutineParam<InferenceContext<B, M, S>> for MultiDevice<B> {
    type Item<'new>
        = MultiDevice<B>
    where
        B: 'new,
        M: 'new,
        S: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M, S>) -> anyhow::Result<Self::Item<'_>> {
        Ok(MultiDevice(ctx.devices.clone()))
    }
}

type ArcInferenceHandler<B, M, I, S> =
    Arc<dyn Routine<InferenceContext<B, M, S>, In = I, Out = ()>>;

pub struct InferenceState<M> {
    model: Mutex<M>,
}

pub struct Inference<B: Backend, M, I, O> {
    pub id: String,
    state: Arc<InferenceState<M>>,
    handler: ArcInferenceHandler<B, M, I, O>,
    burn_central: BurnCentral,
    namespace: String,
    project: String,
    phantom_data: PhantomData<O>,
}

impl<B, M, I, O> Inference<B, M, I, O>
where
    B: Backend,
    M: Send + 'static,
    I: RoutineInput + 'static,
    O: Send + 'static,
{
    fn new(
        id: String,
        handler: ArcInferenceHandler<B, M, I, O>,
        model: M,
        client: BurnCentral,
        namespace: String,
        project: String,
    ) -> Self {
        Self {
            id,
            state: Arc::new(InferenceState {
                model: model.into(),
            }),
            handler,
            burn_central: client,
            namespace,
            project,
            phantom_data: Default::default(),
        }
    }

    pub fn infer(
        &self,
        input: I::Inner<'_>,
        devices: impl IntoIterator<Item = B::Device>,
    ) -> Result<Vec<O>, InferenceError> {
        let collector = Arc::new(CollectEmitter::new());
        {
            let mut ctx = InferenceContext {
                id: self.id.clone(),
                devices: devices.into_iter().collect(),
                state: self.state.clone(),
                emitter: collector.clone(),
                cancel_token: Arc::new(AtomicBool::new(false)),
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

    pub fn spawn(
        &self,
        input: I::Inner<'static>,
        devices: impl IntoIterator<Item = B::Device>,
    ) -> job::JobHandle<O>
    where
        <I as RoutineInput>::Inner<'static>: Send,
    {
        let id = self.id.clone();
        let (stream_tx, stream_rx) = std::sync::mpsc::sync_channel(10);
        let cancel_token = CancelToken::new();
        let mut ctx = InferenceContext {
            id,
            devices: devices.into_iter().collect(),
            state: self.state.clone(),
            emitter: Arc::new(SyncChannelEmitter::new(stream_tx)),
            cancel_token: cancel_token.0.clone(),
        };
        let handler = self.handler.clone();
        let id = self.id.clone();
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
    registry: ModelRegistry,
    namespace: String,
    project: String,
    phantom_data: PhantomData<B>,
}

impl<B: Backend> InferenceBuilder<B> {
    pub fn new(
        client: BurnCentral,
        namespace: impl Into<String>,
        project: impl Into<String>,
    ) -> Self {
        let namespace = namespace.into();
        let project = project.into();
        let registry = client.model_registry(&namespace, &project);
        Self {
            client,
            registry,
            namespace,
            project,
            phantom_data: Default::default(),
        }
    }

    pub fn load<M: ModelProvider<B>>(
        self,
        model_spec: ModelSpec,
        device: &B::Device,
    ) -> Result<LoadedInferenceBuilder<B, M>, InferenceError> {
        let model = M::get_model(&self.registry, model_spec, device)
            .map_err(InferenceError::ModelLoadingFailed)?;
        Ok(LoadedInferenceBuilder {
            client: self.client,
            model,
            namespace: self.namespace,
            project: self.project,
            phantom_data: Default::default(),
        })
    }
}
pub struct LoadedInferenceBuilder<B: Backend, M> {
    client: BurnCentral,
    model: M,
    namespace: String,
    project: String,
    phantom_data: PhantomData<B>,
}

impl<B, M> LoadedInferenceBuilder<B, M>
where
    B: Backend,
    M: Send + 'static,
{
    pub fn build<'a, F, I, O, Marker>(self, handler: F) -> Inference<B, M, I, O>
    where
        F: IntoRoutine<InferenceContext<B, M, O>, I, (), Marker>,
        I: RoutineInput + 'static,
        O: Send + 'static,
    {
        Inference::new(
            crate::type_name::fn_type_name::<F>(),
            Arc::new(IntoRoutine::into_routine(handler)),
            self.model,
            self.client,
            self.namespace,
            self.project,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{In, MultiDevice};
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
        model: Model<MyNnModel<B>>,
        devices: MultiDevice<B>,
        output: Out<Tensor<B, 2>>,
    ) {
        println!("Running inference with model: {:?}", *model);
        println!("Using devices: {:?}", devices[0]);
        println!("Input tensor: {}", input);
        let input = input;
        for i in 0..5 {
            let result = model.0.linear.forward(input.clone());
            if output.emit(result).is_err() {
                println!("Emitter requested to stop, exiting inference function.");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }

    fn panicking_inference_function<B: Backend>(
        input: In<Tensor<B, 2>>,
        model: Model<MyNnModel<B>>,
        devices: MultiDevice<B>,
        output: Out<String>,
    ) {
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

    static NAMESPACE: &str = "default-namespace";
    static PROJECT: &str = "default-project";

    #[test]
    fn test_inference_creation() {
        let client = provision_client();
        let namespace = NAMESPACE;
        let project = PROJECT;

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client, namespace, project)
            .load::<MyNnModel<TestBackend>>("my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(my_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        println!("{}", serde_json::to_string(&input).unwrap());
        let output = inference.infer(input, vec![device]).unwrap();
        println!("Inference output: {:?}", output);
    }

    #[test]
    fn test_inference_job_spawn() {
        let client = provision_client();
        let namespace = NAMESPACE;
        let project = PROJECT;

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client, namespace, project)
            .load::<MyNnModel<TestBackend>>("my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(my_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        println!("{}", serde_json::to_string(&input).unwrap());
        let job = inference.spawn(input, vec![device]);
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
        let namespace = NAMESPACE;
        let project = PROJECT;

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client, namespace, project)
            .load::<MyNnModel<TestBackend>>("my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(panicking_inference_function);

        let input = Tensor::<TestBackend, 2>::ones([1, 10], &device);
        println!("{}", serde_json::to_string(&input).unwrap());
        let job = inference.spawn(input, vec![device]);
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
}
