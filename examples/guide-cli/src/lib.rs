//
// Note: If you are following the Burn Book guide this file can be ignored.
//
// This lib.rs file is added only for convenience so that the code in this
// guide can be reused.
//
pub mod data;
pub mod inference;
pub mod model;
pub mod training;

pub use burn;

pub mod guide_mod {
    use tracel::heat::{client::HeatClient, heat};

    use burn::optim::AdamConfig;
    use burn::tensor::backend::AutodiffBackend;

    pub use crate::model::{Model, ModelConfig};
    pub use crate::training::{self, TrainingConfig};

    #[heat(training)]
    pub fn training<B: AutodiffBackend>(
        mut client: HeatClient,
        devices: Vec<B::Device>,
        config: TrainingConfig,
    ) -> Result<Model<B>, ()> {
        training::train::<B>(&mut client, "/tmp/guide", config, devices[0].clone())
    }
}
