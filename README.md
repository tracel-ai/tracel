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

Wrap your training function into a job with `Experiments::create`, then register it with a
`Cli` (to run it from the command line) or a `Server` (to dispatch it over HTTP):

```rust
use tracel::Target;
use tracel::app::cli::Cli;
use tracel::app::cli::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;

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
    let experiments = Target::from_env()?.experiments()?;
    let job = experiments.create("mnist", training);

    Cli::new()
        .register(job, JsonMapper::with_default(YourExperimentConfig::default()))
        .run()?;

    Ok(())
}
```

Swap `Cli` for `tracel::app::server::Server` (with the optional `server` feature) to dispatch the
same job over HTTP instead of the command line. The [`cli`](examples/basics/examples/cli.rs) and
[`serve`](examples/basics/examples/serve.rs) examples in [`examples/basics`](examples/basics) are
complete, runnable versions of both.

`Target::from_env` records offline under `./runs` unless `TRACEL_CONNECTION` says otherwise:

| Variable | Value | Default |
| --- | --- | --- |
| `TRACEL_CONNECTION` | `offline` or `console` | `offline` |
| `TRACEL_RUNS_DIR` | the directory offline runs are recorded under | `./runs` |
| `TRACEL_ENV` | the console to reach | `Production` |
| `TRACEL_API_KEY` | an API key or a job token | the stored `tracel login` session |
| `TRACEL_NAMESPACE`, `TRACEL_PROJECT` | the console project | `namespace` and `project` in `tracel.toml` |

Without a `Target`, build the services directly: `tracel::console::ProjectHandle::from_env()?.experiments()`
for a console project, or `tracel::experiment::local::LocalExperiments` to record offline.

### 2. Integrate with your Learner

To track a Burn training run, enable the `burn` feature of `tracel` and wire the run into your
`SupervisedTraining` with `ExperimentTrainingExt`:

```rust
use burn::train::{Learner, SupervisedTraining, metric::{AccuracyMetric, LossMetric}};
use tracel::experiment::integration::training::ExperimentTrainingExt;

let (model_checkpointer, optimizer_checkpointer, scheduler_checkpointer) =
    experiment.checkpointers();

let result = SupervisedTraining::new(artifact_dir, dataloader_train, dataloader_valid)
    .metrics((AccuracyMetric::new(), LossMetric::new()))
    .num_epochs(config.num_epochs)
    .summary()
    // Experiment metric logging
    .with_metric_logger(experiment.metric_logger())
    // Epoch and split progress as experiment activities
    .with_progress_logger(experiment.training_progress_logger())
    // Experiment checkpoint saving
    .with_custom_checkpointers(model_checkpointer, optimizer_checkpointer, scheduler_checkpointer)
    // Experiment interruption handling
    .with_interrupter(experiment.interrupter())
    .launch(Learner::new(model, optimizer, lr_scheduler));
```

`SupervisedTrainingExperimentExt` does the same in two calls:
`.with_experiment(experiment).with_experiment_checkpoints(experiment)`. See
[`examples/mnist/src/training.rs`](examples/mnist/src/training.rs) for the complete wiring.

### 3. Run your training

Once integrated, run your training by running your binary (`cargo run`). It records metrics,
checkpoints, and logs under `./runs`; run it with `TRACEL_CONNECTION=console`, after `tracel login`,
to track them on the console instead.

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
