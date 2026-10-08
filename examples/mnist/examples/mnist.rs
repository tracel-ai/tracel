//! Train MNIST with the Burn `train` integration: metrics, checkpoints, progress, and cancellation.
//! See src/training.rs for the wiring.
//!
//! cargo run -p mnist --example mnist
//! cargo run -p mnist --example mnist -- mnist --help
//! cargo run -p mnist --example mnist -- mnist --num-epochs 5 --optimizer.weight-decay 0.0001
//! cargo run -p mnist --example mnist -- mnist '{"num_epochs": 5}'
#![recursion_limit = "256"]

use std::process::ExitCode;

use burn::backend::wgpu::WgpuDevice;
use burn::tensor::Device;
use mnist::training::{self, MnistTrainingConfig};

use tracel::Target;
use tracel::runtime::{BoxError, Job, Jobs};

/// Trains the model in an experiment, which records the config as its arguments.
fn mnist(config: MnistTrainingConfig) -> Result<(), BoxError> {
    let experiments = Target::from_env()?.experiments()?;
    experiments.run("mnist", |experiment| {
        experiment.log_args(&config)?;
        training::run(
            experiment,
            config,
            vec![Device::autodiff(WgpuDevice::default().into())],
        )
    })
}

fn main() -> ExitCode {
    Jobs::new()
        .version(env!("CARGO_PKG_VERSION"))
        .job(
            "mnist",
            Job::new(mnist)
                .with_description("Train an MNIST classifier")
                .with_default(MnistTrainingConfig::small()),
        )
        .default_job("mnist")
        .run()
}
