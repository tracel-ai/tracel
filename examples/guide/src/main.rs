use clap::Parser;
use tracel::heat::client::HeatClient;
mod data;
mod inference;
mod model;
mod training;

use crate::{model::ModelConfig, training::TrainingConfig};
use burn::{
    backend::{Autodiff, Wgpu},
    data::dataset::Dataset,
    optim::AdamConfig,
};

fn main() {
    type MyBackend = Wgpu<f32, i32>;
    type MyAutodiffBackend = Autodiff<MyBackend>;

    let args = Args::parse();
    let mut heat = heat_client(&args.key, &args.url, &args.project);
    let device = burn::backend::wgpu::WgpuDevice::default();
    let artifact_dir = "/tmp/guide";
    crate::training::train::<MyAutodiffBackend>(
        &mut heat,
        artifact_dir,
        TrainingConfig::new(ModelConfig::new(10, 512), AdamConfig::new()),
        device.clone(),
    );
    crate::inference::infer::<MyBackend>(
        artifact_dir,
        device,
        burn::data::dataset::vision::MnistDataset::test()
            .get(42)
            .unwrap(),
    );
}

#[derive(Parser, Debug)]
#[command(name = "Guide")]
#[command(about = "Example to train a model and make prediction using Burn and Heat.", long_about = None)]
struct Args {
    /// The API key necessary to connect to the Heat server.
    #[arg(short, long)]
    key: String,

    /// Base URL of the Heat server.
    #[arg(short, long, default_value = "http://localhost:9001")]
    url: String,

    /// The project ID in which the experiment will be created.
    #[arg(short, long)]
    project: String,
}

fn heat_client(api_key: &str, url: &str, project: &str) -> HeatClient {
    let creds = tracel::heat::client::HeatCredentials::new(api_key.to_owned());
    let client_config = tracel::heat::client::HeatClientConfig::builder(
        creds,
        tracel::heat::schemas::ProjectPath::try_from(project.to_string())
            .expect("Project path should be valid."),
    )
    .with_endpoint(url)
    .with_num_retries(10)
    .build();
    tracel::heat::client::HeatClient::create(client_config)
        .expect("Should connect to the Heat server and create a client")
}
