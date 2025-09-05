use super::errors::ModelProviderResult;
use burn::prelude::Backend;
use burn_central_client::model::{ModelRegistry, ModelSpec};

pub trait ModelProvider<B: Backend>: Sized {
    fn get_model(
        registry: &ModelRegistry,
        model_spec: ModelSpec,
        device: &B::Device,
    ) -> ModelProviderResult<Self>;
}
