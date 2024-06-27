use guide_cli::{model::Model, training};
use tracel::heat::{client::HeatClient, heat};

use burn::{optim::AdamConfig, tensor::backend::AutodiffBackend};
use guide_cli::training::TrainingConfig;

#[heat(training)]
fn training<B: AutodiffBackend>(
    mut client: HeatClient,
    devices: Vec<B::Device>,
    config: TrainingConfig,
) -> Result<Model<B>, ()> {
    training::train::<B>(&mut client, "/tmp/guide", config, devices[0].clone())
}

fn main() {
    heat_main();
}
