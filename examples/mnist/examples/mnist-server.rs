#![recursion_limit = "256"]

use burn::backend::{FlexDevice, wgpu::WgpuDevice};
use burn::tensor::Device;
use mnist::training::{self, MnistTrainingConfig};

use tracel::app::server::{JsonBody, Server};
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

    let no_default_job = module.create("mnist_no_default", |session: &ExperimentRun, config| {
        training::run(
            session,
            config,
            vec![Device::autodiff(WgpuDevice::default().into())],
        )
    });

    Server::new()
        .port(3000)
        .register(job, JsonBody::with_default(MnistTrainingConfig::default()))
        .register(default_job, JsonBody::with_default(MnistTrainingConfig::small()))
        .register(no_default_job, JsonBody::new())
        .run()?;

    Ok(())
}
