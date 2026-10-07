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
use std::process::ExitCode;

use tracel::Target;
use tracel::app::cli::Cli;
use tracel::app::mapper::JsonMapper;
use tracel::experiment::ExperimentRun;

fn training(
    experiment: &ExperimentRun,
    config: YourExperimentConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Your training logic here...
    train(experiment, &config)?;

    Ok(())
}

fn main() -> anyhow::Result<ExitCode> {
    let experiments = Target::from_env()?.experiments()?;
    let job = experiments
        .create("mnist", training)
        .with_description("Train the MNIST classifier");

    Ok(Cli::new()
        .version(env!("CARGO_PKG_VERSION"))
        .register(job, JsonMapper::with_default(YourExperimentConfig::default()))
        .run())
}
```

The binary runs a job by name, with its input as flags or one JSON document merged onto the
default: `cargo run -- mnist --num-epochs 5` or `cargo run -- mnist '{"num_epochs": 5}'`. Left
out, the input is the default. The experiment records the merged input as its arguments, which the
console lists as its config. `version` gives the binary its `--version`, which prints the binary's
name and that version, such as `mnist 0.1.0`; without it, the binary has no `--version`.

Swap `Cli` for `tracel::app::server::Server` (with the optional `server` feature) to dispatch the
same job over HTTP instead of the command line. The [`cli`](examples/basics/examples/cli.rs) and
[`serve`](examples/basics/examples/serve.rs) examples in [`examples/basics`](examples/basics) are
complete, runnable versions of both.

`Target::from_env` records offline under `./runs` unless `TRACEL_TARGET` says otherwise:

| Variable | Value | Default |
| --- | --- | --- |
| `TRACEL_TARGET` | `offline` or `console` | `offline` |
| `TRACEL_RUNS_DIR` | the directory offline runs are recorded under | `./runs` |
| `TRACEL_ENV` | the console to reach | `Production` |
| `TRACEL_API_KEY` | an API key or a job token | the stored `tracel login` session |
| `TRACEL_NAMESPACE`, `TRACEL_PROJECT` | the console project | `namespace` and `project` in `tracel.toml` |

Without a `Target`, build the services directly: `tracel::console::ProjectHandle::from_env()?.experiments()`
for a console project, or `tracel::experiment::local::LocalExperiments` to record offline.

#### Command-line flags

`Cli` builds its command line from the definitions of its jobs, so each field of a job's input is
also a flag: `cargo run -- mnist --num-epochs 5` runs with the same input as
`cargo run -- mnist '{"num_epochs": 5}'`. `--help` lists the jobs, and `mnist --help` the job's
flags with their defaults:

```text
$ cargo run -- mnist --help
Train the MNIST classifier

Usage: mnist mnist [OPTIONS] [INPUT]

Arguments:
  [INPUT]  JSON merged after --config and before the flags

Options:
      --batch-size <INT>                [default: 64]
      --num-epochs <INT>                [default: 10]
      --optimizer.lr <FLOAT>            [default: 0.001]
      --optimizer.weight-decay <FLOAT>  [default: 0.00005]
  -c, --config <FILE>                   JSON merged before the input and the flags
  -h, --help                            Print help
```

- With an input schema (`JsonMapper::with_schema` and the `schema` feature), a flag takes its
  field's type, allowed values and requiredness from the schema, and the field's doc comment as
  its help. Without one, each field of the example input is a flag typed by its value: a boolean,
  an integer, a number or a string. Burn `Config` types work this way.
- A nested field's flag joins the keys with dots and writes `_` as `-`: the field
  `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, an object with
  no fields of its own, or `null` in the example input takes a JSON literal, such as
  `--layers '[64, 32]'`. A boolean flag given no value is `true`.
- The input is the example input, then the `--config` file, then the JSON document, then the
  flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
- `help`, `version` and `config` are the runner's names: a field with one of them has no flag, and
  is set through `--config` or the JSON document.
- `--completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`, `powershell`
  or `zsh`: `cargo run -- --completions bash > mnist.bash`.

`tracel::job::command` builds the same command line from job definitions alone, such as those of
a definitions file. A job whose input type derives `clap::Parser` can parse its own arguments
instead, through `ClapMapper`, from a JSON string: `cargo run -- <job> '"--epochs 3"'`.

#### Describing jobs

With `TRACEL_DESCRIBE=<path>` set, `run()` writes the definitions of the binary's jobs to `<path>`
and returns successfully without running a job or serving. `Cli` and `Server` both do this, writing
the file to `<path>.tmp` first and renaming it, so it is never read half written:

```json
{
  "protocol": 1,
  "sdk_version": "0.10.0",
  "runner": "cli",
  "jobs": [
    {
      "name": "mnist",
      "kind": "experiment",
      "description": "Train the MNIST classifier",
      "input_schema": null,
      "input_example": { "num_epochs": 10, "optimizer": { "lr": 0.001 } }
    }
  ]
}
```

`kind` is `experiment` or `inference`. `input_example` is the default given to
`JsonMapper::with_default`. `input_schema` is the input type's JSON Schema when the mapper is built
with `JsonMapper::with_schema`, which needs the `schema` feature and an input type that derives
`schemars::JsonSchema`. While `TRACEL_DESCRIBE` is set, `ExperimentJob::run` returns an error
without creating an experiment, so describing a binary never trains.

#### Launching jobs

A binary whose `main` returns `Cli::run()` can be launched by another program, such as the
`tracel` CLI, which reads its exit code:

| Exit code | Meaning |
| --- | --- |
| 0 | The job completed, the help, version or completion script was printed, or the definitions file was written |
| 1 | The job failed |
| 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
| 130 | The job was asked to stop |

With `TRACEL_REPORT_FILE=<path>` set, an experiment job writes a run report to `<path>` when it
creates its experiment, and again when the run ends, each time to `<path>.tmp` first, renamed to
`<path>`. Without it, nothing is written:

```json
{
  "protocol": 1,
  "job": "mnist",
  "experiment": { "num": 42, "url": "https://console.tracel.ai/users/me/projects/demo/experiments/42" },
  "status": "completed",
  "started_at": "2026-10-06T14:02:11Z",
  "finished_at": "2026-10-06T14:31:40Z",
  "error": null
}
```

`status` is `running`, `completed` or `failed`, and `error` says why a run failed.
`url` is the experiment's page on the console; offline it is `null`, and `dir` gives the run's
directory instead, where `status.json` holds the same report and `events.jsonl` the run's events,
one JSON object per line.

With `TRACEL_JOB_NUM` set, the experiment records it as its `tracel.job_num` attribute, which links
it to the job that ran it.

SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop: it cancels the
experiment's cancel token, which stops a learner given `experiment.interrupter()`, and the
experiment logs a warning that a stop was requested. The run ends as `completed` or `failed` by
what the job's function returns, and the binary exits with code 130. A second signal ends the
binary at once. Launchers send SIGKILL after a 30-second grace period.

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

Once integrated, run your training by running your binary (`cargo run -- mnist`). It records metrics,
checkpoints, and logs under `./runs`; run it with `TRACEL_TARGET=console`, after `tracel login`, to
track them on the console instead.

## Requirements

- Rust 1.97.0 or higher
- A Burn Central account (create one at [central.burn.dev](https://central.burn.dev/))
- The [tracel-cli](https://github.com/tracel-ai/tracel-cli), to log in and store your credentials locally

## Contribution

Contributions to this repository are welcome. You can also submit issues for features you would like to see in the near future.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
