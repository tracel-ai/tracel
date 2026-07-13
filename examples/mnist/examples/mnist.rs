//! Train MNIST with the Burn `train` integration: metrics, checkpoints, progress, and cancellation.
//! See src/training.rs for the wiring.
//!
//! cargo run -p mnist --example mnist
#![recursion_limit = "256"]

use burn::backend::wgpu::WgpuDevice;
use burn::tensor::Device;
use mnist::training::{self, MnistTrainingConfig};

use tracel::experiment::ExperimentRun;

fn main() -> anyhow::Result<()> {
    let module = common::context()?.experiment();

    module
        .create("mnist", |experiment: &ExperimentRun, config| {
            training::run(
                experiment,
                config,
                vec![Device::autodiff(WgpuDevice::default().into())],
            )
        })
        .run(MnistTrainingConfig::small())
        .map_err(|e| anyhow::anyhow!("training failed: {e}"))?;

    Ok(())
}
