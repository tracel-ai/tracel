use crate::param::RoutineParam;
use crate::{IntoRoutine, Model, MultiDevice, Routine};
use burn::prelude::Backend;
use burn_central_client::model::{ModelRegistry, ModelRegistryError, ModelSpec};
use burn_central_client::BurnCentral;
use std::sync::Arc;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InferenceServiceError {
    #[error("Model loading failed: {0}")]
    ModelLoadingFailed(#[from] ModelProviderError),
    #[error("Inference handler execution failed: {0}")]
    HandlerExecutionFailed(anyhow::Error),
}

pub struct InferenceContext<B: Backend, M> {
    pub devices: Vec<B::Device>,
    pub model: Arc<M>,
}

#[derive(Debug, Error)]
pub enum ModelProviderError {
    #[error("Model registry error: {0}")]
    ModelLoadingFailed(#[from] ModelRegistryError),
}
type ModelProviderResult<M> = Result<M, ModelProviderError>;
pub trait ModelProvider: Sized {
    fn get_model(registry: &ModelRegistry, model_spec: ModelSpec) -> ModelProviderResult<Self>;
}

impl<B: Backend, M: Clone> RoutineParam<InferenceContext<B, M>> for Model<M> {
    type Item<'new>
        = Model<M>
    where
        M: 'new;

    fn try_retrieve(ctx: &InferenceContext<B, M>) -> anyhow::Result<Self::Item<'_>> {
        Ok(Model((*ctx.model).clone()))
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

pub struct ServiceState<M> {
    model: M,
}

pub struct InferenceService<B: Backend, M> {
    pub id: String,
    state: Arc<ServiceState<M>>,
    handler: ArcInferenceHandler<B, M>,

    namespace: String,
    project: String,
    burn_central: BurnCentral,
}

pub struct ModelStoreKey {
    id: String,
}

pub trait IntoService<B: Backend, M, Marker> {
    fn into_service(
        self,
        id: String,
        model_spec: ModelSpec,
        client: BurnCentral,
        namespace: String,
        project: String,
    ) -> Result<InferenceService<B, M>, InferenceServiceError>;
}

impl<B, M, F, Marker> IntoService<B, M, Marker> for F
where
    B: Backend,
    M: ModelProvider,
    F: IntoRoutine<InferenceContext<B, M>, (), Marker>,
{
    fn into_service(
        self,
        id: String,
        model_spec: ModelSpec,
        client: BurnCentral,
        namespace: String,
        project: String,
    ) -> Result<InferenceService<B, M>, InferenceServiceError> {
        let registry = client.model_registry(&namespace, &project);
        let model = M::get_model(&registry, model_spec)?;

        let handler = Arc::new(IntoRoutine::into_routine(self));
        let svc = InferenceService::new(id, handler, model, client, namespace, project);

        Ok(svc)
    }
}

impl<B: Backend, M> InferenceService<B, M> {
    pub fn new(
        id: String,
        handler: ArcInferenceHandler<B, M>,
        model: M,
        client: BurnCentral,
        namespace: String,
        project: String,
    ) -> Self {
        Self {
            id,
            state: Arc::new(ServiceState { model }),
            handler,
            namespace,
            project,
            burn_central: client,
        }
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

    impl<B: Backend> ModelProvider for MyNnModel<B> {
        fn get_model(registry: &ModelRegistry, model_spec: ModelSpec) -> ModelProviderResult<Self> {
            println!("Fetching model from registry with spec: {model_spec}");

            let model_artifact = registry.get_model(model_spec)?;
            // Here you would implement the logic to fetch and load the model from the registry
            // For testing purposes, we will just create a new model instance
            let device = &Default::default();
            Ok(MyNnModel::new(device))
        }
    }

    fn my_inference_function<B: Backend>(model: Model<MyNnModel<B>>, devices: MultiDevice<B>) {
        println!("Running inference with model: {:?}", *model);
        println!("Using devices: {:?}", devices[0]);
    }

    #[test]
    fn test_inference_service_creation() {
        let creds = BurnCentralCredentials::from_str("16336384-9c97-4981-a7c6-5ad80f0e70ea")
            .expect("Should be able to create credentials");

        let burn_central = BurnCentral::builder(creds)
            .with_endpoint("http://localhost:9001")
            .build()
            .expect("Should be able to login");
        let model_spec = ModelSpec::new("my_nn_model".to_string(), 1);

        let service = my_inference_function::<TestBackend>.into_service(
            "test-service".to_string(),
            model_spec,
            burn_central,
            "default-namespace".to_string(),
            "default-project".to_string(),
        );

        assert!(service.is_ok());
        let service = service.unwrap();
        assert_eq!(service.id, "test-service");
    }
}
