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
account on the [application](https://s1-central.burn.dev/).

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

Here's how to integrate Tracel into your training workflow:

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

The binary runs a job by name, with its input as flags or one JSON document merged onto the
default: `cargo run -- mnist --num-epochs 5` or `cargo run -- mnist '{"num_epochs": 5}'`. Left
out, the input is the default. A function registered without `Job`, as in
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
number and input, is asked to stop when the job is, and is shown with the job by `tracel run`.
Experiments built another way, such as `ProjectHandle::experiments`, are not linked unless the
code links them, through `Experiments::attribute`, `arguments`, `cancel_token` and `on_start`.

The [`cli`](examples/basics/examples/cli.rs) example in [`examples/basics`](examples/basics) is a
complete, runnable version.

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

`Jobs` builds its command line from the definitions of its jobs, so each field of a job's input is
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

- With an input schema (`Job::with_schema` and the `schema` feature), a flag takes its field's
  type, allowed values and requiredness from the schema, and the field's doc comment as its help.
  Without one, each field of the default input is a flag typed by its value: a boolean, an
  integer, a number or a string. Burn `Config` types work this way.
- A nested field's flag joins the keys with dots and writes `_` as `-`: the field
  `optimizer.weight_decay` is `--optimizer.weight-decay`. A field that is an array, an object with
  no fields of its own, or `null` in the default input takes a JSON literal, such as
  `--layers '[64, 32]'`. A boolean flag given no value is `true`.
- The input is the default input, then the `--config` file, then the JSON document, then the
  flags, each merged onto the one before, so a later one wins. Launchers pass the JSON document.
- `help`, `version` and `config` are the command line's names: a field with one of them has no
  flag, and is set through `--config` or the JSON document.
- `--completions <SHELL>` prints the completion script for `bash`, `elvish`, `fish`, `powershell`
  or `zsh`: `cargo run -- --completions bash > mnist.bash`.

#### Launching jobs

`tracel run <job>` builds the binary and runs one of its jobs, taking the job's flags on its own
command line, and `tracel run --list` lists the jobs with their descriptions. The binary's exit
code says how the job ended:

| Exit code | Meaning |
| --- | --- |
| 0 | The job completed, or the help, version or completion script was printed |
| 1 | The job failed |
| 2 | No job or an unknown job is named, and stderr lists the job names; or a flag or the input is unusable or does not decode |
| 130 | The job was asked to stop |

SIGTERM, SIGINT or SIGHUP, or Ctrl-C on Windows, asks the running job to stop:
`tracel::runtime::stop_requested()` turns `true`, and what `tracel::runtime::on_stop` registered
runs. Each experiment linked to the job has its cancel token cancelled, which stops a learner given
`experiment.interrupter()`, and logs a warning that a stop was requested; none starts after. The
job ends as completed or failed by what it returns, and the binary exits with code 130. A second
signal ends the binary at once, and `tracel run` sends SIGKILL after a 30-second grace period.

Programs that launch job binaries, as `tracel run` does, follow the contract of the
`tracel-job-proto` crate.

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
