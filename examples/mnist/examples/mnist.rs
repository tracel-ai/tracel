//! Train MNIST with the Burn `train` integration.
//!
//! Everything a real learner needs flows to the experiment through `ExperimentTrainingExt`
//! (see `src/training.rs`): metrics via `metric_logger()`, checkpoints via `checkpointers()`,
//! epoch/split progress via `training_progress_logger()`, and cancellation via `interrupter()`.
//!
//! Run: `cargo run -p mnist --example mnist`
#![recursion_limit = "256"]

use burn::backend::wgpu::WgpuDevice;
use burn::tensor::Device;
use mnist::training::{self, MnistTrainingConfig};

use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    // For Cloud (activities, metrics, and checkpoints stream to the dashboard):
    //   let module = Context::new(Connection::Cloud)?.experiment();
    let module = Context::new(Connection::Offline("./runs".into()))?.experiment();

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
