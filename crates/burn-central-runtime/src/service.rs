use crate::param::RoutineParam;
use crate::{IntoRoutine, Model, MultiDevice, Routine};
use burn::prelude::Backend;
use burn_central_client::model::{ModelRegistry, ModelRegistryError, ModelSpec};
use burn_central_client::BurnCentral;
use std::marker::PhantomData;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceError {
    #[error("Model loading failed: {0}")]
    ModelLoadingFailed(#[from] ModelProviderError),
    #[error("Inference handler execution failed: {0}")]
    HandlerExecutionFailed(anyhow::Error),
}

pub struct InferenceContext<B: Backend, M> {
    pub devices: Vec<B::Device>,
    pub state: Arc<InferenceState<M>>,
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

impl<B: Backend, M: Clone + ModelProvider<B>> RoutineParam<InferenceContext<B, M>> for Model<M> {
    type Item<'new>
        = Model<M>
    where
        M: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M>) -> anyhow::Result<Self::Item<'_>> {
        Ok(Model(ctx.state.model.clone()))
    }
}

impl<B: Backend, M> RoutineParam<InferenceContext<B, M>> for MultiDevice<B> {
    type Item<'new>
        = MultiDevice<B>
    where
        B: 'new,
        M: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M>) -> anyhow::Result<Self::Item<'_>> {
        Ok(MultiDevice(ctx.devices.clone()))
    }
}

type ArcInferenceHandler<B, M> = Arc<dyn Routine<InferenceContext<B, M>, Out = ()>>;

pub struct InferenceState<M> {
    model: M,
}

pub struct Inference<B: Backend, M> {
    pub id: String,
    state: Arc<InferenceState<M>>,
    handler: ArcInferenceHandler<B, M>,
    burn_central: BurnCentral,
    namespace: String,
    project: String,
}

impl<B: Backend, M: 'static> Inference<B, M> {
    fn new(
        id: String,
        handler: ArcInferenceHandler<B, M>,
        model: M,
        client: BurnCentral,
        namespace: String,
        project: String,
    ) -> Self {
        Self {
            id,
            state: Arc::new(InferenceState { model }),
            handler,
            burn_central: client,
            namespace,
            project,
        }
    }

    pub fn infer(&self, devices: Vec<B::Device>) -> Result<(), InferenceError> {
        let mut ctx = InferenceContext {
            devices,
            state: self.state.clone(),
        };
        self.handler
            .run(&mut ctx)
            .map_err(|e| InferenceError::HandlerExecutionFailed(e.into()))?;

        Ok(())
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

impl<B: Backend, M: 'static> LoadedInferenceBuilder<B, M> {
    pub fn build<F, Marker>(self, handler: F) -> Inference<B, M>
    where
        F: IntoRoutine<InferenceContext<B, M>, (), Marker>,
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
    use crate::MultiDevice;
    use burn::backend::NdArray;
    use burn::nn::{Linear, LinearConfig};
    use burn::prelude::{Backend, Module};
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

            let device = &Default::default();
            Ok(MyNnModel::new(device))
        }
    }

    fn my_inference_function<B: Backend>(model: Model<MyNnModel<B>>, devices: MultiDevice<B>) {
        println!("Running inference with model: {:?}", *model);
        println!("Using devices: {:?}", devices[0]);
    }

    #[test]
    fn test_inference_creation() {
        let creds = BurnCentralCredentials::from_str("16336384-9c97-4981-a7c6-5ad80f0e70ea")
            .expect("Should be able to create credentials");

        let client = BurnCentral::builder(creds)
            .with_endpoint("http://localhost:9001")
            .build()
            .expect("Should be able to login");

        let namespace = "default-namespace";
        let project = "default-project";

        let device = Device::default();

        let inference = InferenceBuilder::<TestBackend>::new(client, namespace, project)
            .load::<MyNnModel<TestBackend>>("my_nn_model:1".parse().unwrap(), &device)
            .unwrap()
            .build(my_inference_function);

        inference.infer(vec![device]).unwrap();
    }
}
