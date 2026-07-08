#![recursion_limit = "256"]

use burn::backend::{FlexDevice, wgpu::WgpuDevice};
use burn::tensor::Device;
use mnist::training::{self, MnistTrainingConfig};

use tracel::app::cli::Cli;
use tracel::app::cli::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Cloud)?.experiment();
    let job = module.create("mnist_wgpu", |session: &ExperimentRun, config| {
        training::run(
            session,
            config,
            vec![Device::autodiff(WgpuDevice::default().into())],
        )
    });
    let default_job = module.create("mnist_flex", |session: &ExperimentRun, config| {
        training::run(
            session,
            config,
            vec![Device::autodiff(FlexDevice::default().into())],
        )
    });

    Cli::new()
        .register(
            job,
            JsonMapper::with_default(MnistTrainingConfig::default()),
        )
        .default_job(default_job, MnistTrainingConfig::small())
        .run()?;

    Ok(())
}
