# MNIST

Adapts the Burn MNIST example into a Tracel project. It trains a model and reports the experiment
(metrics, checkpoints, progress, and artifacts) to Tracel. This is the only example that uses Burn;
see [`basics`](../basics) for the framework without it.

## Burn `train` integration

`src/training.rs` wires the learner to the experiment through `ExperimentTrainingExt`:

- `metric_logger()` for training and validation metrics
- `checkpointers()` for model, optimizer, and scheduler checkpoints
- `training_progress_logger()` for epoch and split progress as experiment activities
- `interrupter()` for cancellation

## Run

```bash
cargo run -p mnist --example mnist
cargo run -p mnist --example mnist -- mnist --help  # lists the flags and their defaults
cargo run -p mnist --example mnist -- mnist --num-epochs 5 --optimizer.weight-decay 0.0001
cargo run -p mnist --example mnist -- mnist '{"num_epochs": 5}'
```

The example runs its `mnist` job with `tracel::app::cli::Cli`, so each field of
`MnistTrainingConfig`, a Burn `Config`, is a flag typed by its default value, nested fields joined
with dots. The flags, or one JSON document, are merged onto the small default configuration and
recorded as the experiment's arguments.
SIGTERM cancels the run: the interrupter stops training, and the run ends as cancelled.
Launchers send SIGKILL after a 30-second grace period.

Runs offline by default, recording under `./runs`, so it needs no credentials. It reads its target
with `tracel::Target::from_env`: set `TRACEL_TARGET=console` to ship metrics, checkpoints, and
live progress to the [console](https://console.tracel.ai), after authenticating:

```bash
tracel login          # or set TRACEL_API_KEY
TRACEL_TARGET=console cargo run -p mnist --example mnist
```

The namespace and project come from `TRACEL_NAMESPACE` and `TRACEL_PROJECT`, or from the
`namespace` and `project` keys of [`tracel.toml`](tracel.toml) when run from this directory.
Enable a backend with Cargo features (defaults to `wgpu` and `flex`).
