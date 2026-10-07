<div align="center">

<h1>Tracel</h1>

[![Current Crates.io Version](https://img.shields.io/crates/v/tracel)](https://crates.io/crates/tracel)
[![Minimum Supported Rust Version](https://img.shields.io/crates/msrv/tracel)](https://crates.io/crates/tracel)
[![Test Status](https://github.com/tracel-ai/tracel/actions/workflows/ci.yml/badge.svg)](https://github.com/tracel-ai/tracel/actions/workflows/ci.yml)
![license](https://shields.io/badge/license-MIT%2FApache--2.0-blue)

---
</div>


## Description

Tracel is a new way of using Burn. It aims at providing a central platform for experiment tracking, model sharing, and deployment for all Burn users!

This repository contains the SDK associated with the project. It provides a Rust API to register your training and inference routines as jobs and dispatch them from a CLI or an HTTP server, sending training data to our application as they run. To use this project you must first create an account on the [console](https://console.tracel.ai/).

You'll also want the [tracel-cli](https://github.com/tracel-ai/tracel-cli) to log in (`tracel login`) and store your credentials locally.

## Installation

Add Tracel to your `Cargo.toml`, with the `burn` feature for the Burn `train` integration:

```toml
[dependencies]
tracel = { version = "0.11.0", features = ["burn"] }
```

Optional features:

- `burn`: Burn `train` integration (metric logging, checkpoints, progress, interruption).
- `server`: the HTTP server front-end, `tracel::app::server::Server`.
- `station` and `runner`: run against a Tracel Station and serve jobs to its queue.

## Quick Start

Here's how to integrate Tracel into a Burn training workflow:

### 1. Register your training function

Wrap your training function into a job with `ExperimentModule::create`, then register it with a
`Cli` to run it from the command line:

```rust
use tracel::app::cli::Cli;
use tracel::app::cli::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;
use tracel::experiment::integration::tracing::try_init_tracing_subscriber;
use tracel::{Connection, Context};

fn training(
    experiment: &ExperimentRun,
    config: YourExperimentConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Log the configuration this run was started with
    experiment.log_args(&config)?;

    // Your training logic here...
    train(experiment, &config)?;

    Ok(())
}

fn main() -> anyhow::Result<()> {
    // Forward `tracing` logs to the running experiment
    let _ = try_init_tracing_subscriber();

    let module = Context::new(Connection::Cloud)?.experiment();
    let job = module.create("mnist", training);

    Cli::new()
        .register(job, JsonMapper::with_default(YourExperimentConfig::default()))
        .run()?;

    Ok(())
}
```

`Connection::Cloud` reads your credentials from `tracel login` (or `TRACEL_API_KEY`) and the
project to report to from a `tracel.toml` in the working directory (or `TRACEL_NAMESPACE` and
`TRACEL_PROJECT`):

```toml
namespace = "your-namespace"
project = "your-project"
```

Use `Connection::Offline("./runs".into())` instead to record runs locally, without an account.

To dispatch the same job over HTTP, register it with a `Server` (requires the `server` feature),
which decodes request bodies with `JsonBody` and serves each job at `POST /{name}`:

```rust
use tracel::app::server::{JsonBody, Server};

Server::new()
    .port(3000)
    .register(job, JsonBody::with_default(YourExperimentConfig::default()))
    .run()?;
```

See [`examples/basics`](examples/basics) for complete, runnable `cli` and `serve` examples.

### 2. Integrate with your Learner

To enable experiment tracking, attach the experiment to your `SupervisedTraining`:

```rust
use burn::train::{Learner, SupervisedTraining, metric::{AccuracyMetric, LossMetric}};
use tracel::experiment::integration::training::SupervisedTrainingExperimentExt;

let result = SupervisedTraining::new(artifact_dir, dataloader_train, dataloader_valid)
    .metrics((AccuracyMetric::new(), LossMetric::new()))
    .num_epochs(config.num_epochs)
    .summary()
    // Experiment metric logging, progress tracking, and interruption handling
    .with_experiment(experiment)
    // Experiment model, optimizer, and scheduler checkpoints
    .with_experiment_checkpoints(experiment)
    .launch(Learner::new(model, config.optimizer.init(), learning_rate));
```

For finer control, such as resuming from a previous experiment's checkpoints with
`checkpointers_from`, `ExperimentTrainingExt` builds each adapter separately (`metric_logger()`,
`checkpointers()`, `training_progress_logger()`, `interrupter()`). See
[`examples/mnist`](examples/mnist/src/training.rs) for a complete training run.

### 3. Run your training

Run your binary with the job's name, optionally followed by a JSON config merged over the default,
to track metrics, checkpoints, and logs on the console:

```bash
cargo run -- mnist
cargo run -- mnist '{"num_epochs": 5}'
```

## Requirements

- Rust 1.87.0 or higher
- A Tracel account (create one on the [console](https://console.tracel.ai/))
- The [tracel-cli](https://github.com/tracel-ai/tracel-cli), to log in and store your credentials locally (or set `TRACEL_API_KEY`)

## Contribution

Contributions to this repository are welcome. You can also submit issues for features you would like to see in the near future.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
