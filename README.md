<div align="center">

<h1>Tracel</h1>

[![Current Crates.io Version](https://img.shields.io/crates/v/burn-central)](https://crates.io/crates/burn-central)
[![Minimum Supported Rust Version](https://img.shields.io/crates/msrv/burn-central)](https://crates.io/crates/burn-central)
[![Test Status](https://github.com/tracel-ai/tracel/actions/workflows/ci.yml/badge.svg)](https://github.com/tracel-ai/tracel/actions/workflows/ci.yml)
![license](https://shields.io/badge/license-MIT%2FApache--2.0-blue)

---
</div>


## Description

Tracel is a new way of using Burn. It aims at providing a central platform for experiment tracking, model sharing, and deployment for all Burn users!

This repository contains the SDK associated with the project. It provides a Rust API to register your training and inference routines as jobs and dispatch them from a CLI or an HTTP server, sending training data to our application as they run. To use this project you must first create an account on the [application](https://s1-central.burn.dev/).

You'll also want the [tracel-cli](https://github.com/tracel-ai/tracel-cli) to log in and store your credentials locally.

## Installation

Add Tracel to your `Cargo.toml`:

```toml
[dependencies]
tracel = "0.6.0"
```

## Quick Start

Currently, we only support training. Here's how to integrate Tracel into your training workflow:

### 1. Register your training function

Wrap your training function into a job with `ExperimentModule::create`, then register it with a
`Cli` (to run it from the command line) or a `Server` (to dispatch it over HTTP):

```rust
use tracel::app::cli::Cli;
use tracel::app::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;
use tracel::{Connection, Context};

fn training(
    experiment: &ExperimentRun,
    config: YourExperimentConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Log your configuration
    experiment.log_config("Training Config", &config)
        .expect("Logging config failed");

    // Your training logic here...
    train(experiment, &config)?;

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let module = Context::new(Connection::Cloud)?.experiment();
    let job = module.create("mnist", training);

    Cli::new()
        .register(job, JsonMapper::with_default(YourExperimentConfig::default()))
        .run()?;

    Ok(())
}
```

Swap `Cli` for `tracel::app::server::Server` (with the optional `server` feature) to dispatch the
same job over HTTP instead of the command line. See [`examples/mnist`](examples/mnist/examples)
for complete, runnable versions of both.

### 2. Integrate with your Learner

To enable experiment tracking, add the training integrations to your `LearnerBuilder` and install
the tracing subscriber:

```rust
use burn_central::experiment::integration::training::{
    ExperimentCheckpointRecorder,
    ExperimentMetricLogger,
    experiment_interrupter,
};
use burn_central::experiment::integration::tracing::try_init_tracing_subscriber;
use burn::train::{LearnerBuilder, metric::{AccuracyMetric, LossMetric}};

let _ = try_init_tracing_subscriber();

let learner = LearnerBuilder::new(artifact_dir)
    .metric_train_numeric(AccuracyMetric::new())
    .metric_valid_numeric(AccuracyMetric::new())
    .metric_train_numeric(LossMetric::new())
    .metric_valid_numeric(LossMetric::new())
    // Experiment metric logging
    .with_metric_logger(ExperimentMetricLogger::new(experiment))
    // Experiment checkpoint saving
    .with_file_checkpointer(ExperimentCheckpointRecorder::new(experiment))
    // Experiment interruption handling
    .with_interrupter(experiment_interrupter(experiment))
    .num_epochs(config.num_epochs)
    .summary()
    .build(
        model.init::<B>(&device),
        optimizer.init(),
        learning_rate,
        LearningStrategy::SingleDevice(device),
    );
```

### 3. Run your training

Once integrated, run your training by running your binary (`cargo run`) to automatically track metrics, checkpoints, and logs on Burn Central.

## Requirements

- Rust 1.87.0 or higher
- A Burn Central account (create one at [central.burn.dev](https://central.burn.dev/))
- The [tracel-cli](https://github.com/tracel-ai/tracel-cli), to log in and store your credentials locally

## Contribution

Contributions to this repository are welcome. You can also submit issues for features you would like to see in the near future.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
