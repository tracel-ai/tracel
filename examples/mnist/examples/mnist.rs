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
use tracel::app::cli::Cli;
use tracel::app::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;

fn main() -> anyhow::Result<ExitCode> {
    let train = Target::from_env()?
        .experiments()?
        .create("mnist", |experiment: &ExperimentRun, config| {
            training::run(
                experiment,
                config,
                vec![Device::autodiff(WgpuDevice::default().into())],
            )
        })
        .with_description("Train an MNIST classifier");

    Ok(Cli::new()
        .version(env!("CARGO_PKG_VERSION"))
        .register(
            train,
            JsonMapper::with_default(MnistTrainingConfig::small()),
        )
        .default_job("mnist")
        .run())
}
