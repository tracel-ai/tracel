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

This repository contains the SDK associated with the project. It gives your program clients for
the platform's services, picked from the environment, and one job protocol: the functions your
program registers as jobs run from the command line, locally or launched by the platform, sending
training data to our application as they run. To use this project you must first create an
account on the [console](https://console.tracel.ai/).

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

### 1. Register your training function as a job

A job is a plain function of its typed input. Register it by name with `Jobs`, and return
`Jobs::run` from `main`. The job tracks its training in an experiment it runs with the
experiments `Target` picks from the environment:

```rust
use std::process::ExitCode;

use tracel::Target;
use tracel::runtime::{BoxError, Job, Jobs};

fn training(config: YourExperimentConfig) -> Result<(), BoxError> {
    let experiments = Target::from_env()?.experiments()?;
    experiments.run("mnist", |experiment| {
        // Your training logic here...
        train(experiment, &config)
    })
}

fn main() -> ExitCode {
    Jobs::new()
        .version(env!("CARGO_PKG_VERSION"))
        .job(
            "mnist",
            Job::new(training)
                .with_description("Train the MNIST classifier")
                .with_default(YourExperimentConfig::default()),
        )
        .run()
}
```

The binary runs a job by name, with its input as one JSON document merged onto the default:
`cargo run -- mnist '{"num_epochs": 5}'`, or as flags when the job has an input schema, as
[Command-line flags](#command-line-flags) describes. Left out, the input is the default. A function registered without `Job`, as in
`.job("evaluate", evaluate)`, is a job with no description or default; a job with no input takes
`()`. `version` gives the binary its `--version`, which prints the binary's name and that version,
such as `mnist 0.1.0`; without it, the binary has no `--version`.

`Experiments::run` ends the experiment by what its code returns: completed on `Ok`, and failed
with the error on `Err` or with the message of a panic. `Experiments::start` returns the run
instead, for code that ends it with `finish()` or `fail()`; a run dropped before either ends as
failed. An experiment started in a job, from the experiments a `Target` built, records the job's
input, merged onto its default, as its arguments, which the console lists as its config;
`log_args` replaces them. A job starts as many experiments as its code does, or none.

The experiments `Target::experiments` builds in a job, on the thread that runs it, are linked to
that job, even for runs started on other threads: each one started from them records the job's
number and input, and prints where it records to stderr as it starts, such as
`Recording experiment 42: <url>`, which `tracel run` and a remote job's logs show. Experiments
built another way, such as `ProjectHandle::experiments`, are not linked unless the code links
them, through `Experiments::attribute`, `arguments` and `on_start`.

The [`cli`](examples/basics/examples/cli.rs) example in [`examples/basics`](examples/basics) is a
complete, runnable version.

`Target::from_env` records offline under `./runs` unless `TRACEL_TARGET` says otherwise:

| Variable | Value | Default |
| --- | --- | --- |
| `TRACEL_TARGET` | `offline`, `console`, or `station` with the `station` feature of `tracel` | `offline` |
| `TRACEL_RUNS_DIR` | the directory offline runs are recorded under | `./runs` |
| `TRACEL_ENV` | the console to reach | `Production` |
| `TRACEL_API_KEY` | an API key or a job token | the stored `tracel login` session |
| `TRACEL_NAMESPACE`, `TRACEL_PROJECT` | the console project | `namespace` and `project` in `tracel.toml` |

Without a `Target`, build the services directly: `tracel::console::ProjectHandle::from_env()?.experiments()`
for a console project, or `tracel::experiment::local::LocalExperiments` to record offline.

#### Command-line flags

A job given its input's JSON Schema has a flag per field of its input. Enable the `schema` feature
of `tracel`, derive `schemars::JsonSchema` for the job's input type, and add `with_schema()` to the
job:

```rust
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
struct TrainingConfig {
    /// Passes over the toy dataset.
    epochs: usize,
    /// Batches in each epoch.
    batches_per_epoch: usize,
}

Job::new(toy_training)
    .with_description("Run a toy training loop")
    .with_default(TrainingConfig::default())
    .with_schema()
```

`cargo run -- toy-training --epochs 2` then runs with the same input as
`cargo run -- toy-training '{"epochs": 2}'`. `--help` lists the jobs, and `toy-training --help`
the job's flags with their defaults:

```text
$ cargo run -p basics --example cli -- toy-training --help
Run a toy training loop

Usage: cli toy-training [OPTIONS] [INPUT]

Arguments:
  [INPUT]  JSON merged after --config and before the flags

Options:
      --batches-per-epoch <INT>  Batches in each epoch. [default: 8]
      --epochs <INT>             Passes over the toy dataset. [default: 3]
  -c, --config <FILE>            JSON merged before the input and the flags
  -h, --help                     Print help
```

- A flag takes its field's type, allowed values and requiredness from the schema, and the field's
  doc comment as its help. A job without a schema has no flags: its input is the JSON document or
  the `--config` file.
- A nested field's flag joins the keys with dots and writes `_` as `-`: the field
  `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, a map, an
  optional object or an enum with data takes a JSON literal, such as `--layers '[64, 32]'`. A
  boolean flag given no value is `true`, and takes a value only as `--flag=value`, such as
  `--shuffle=false`.
- The input is the default input, then the `--config` file, then the JSON document, then the
  flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
- `help` and `config` are the command line's names: a field with one of them has no flag, and is
  set through `--config` or the JSON document.
- `--completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`, `powershell`
  or `zsh`: `cargo run -- --completions bash > cli.bash`.

#### Launching jobs

`tracel run <job>` builds the binary and runs one of its jobs, taking the job's flags on its own
command line, and `tracel run --list` lists the jobs with their descriptions. The binary's exit
code says how the job ended:

| Exit code | Meaning |
| --- | --- |
| 0 | The job completed, or the help, version or completion script was printed |
| 1 | The job failed |
| 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |

Programs that launch job binaries, as `tracel run` does, follow the protocol of the `tracel-job`
crate, and read a job's flags as its binary does with its `cli` feature.

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
- A Tracel account (create one on the [console](https://console.tracel.ai/))
- The [tracel-cli](https://github.com/tracel-ai/tracel-cli), to log in and store your credentials locally (or set `TRACEL_API_KEY`)

## Contribution

Contributions to this repository are welcome. You can also submit issues for features you would like to see in the near future.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
