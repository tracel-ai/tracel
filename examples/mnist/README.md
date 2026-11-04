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
cargo run -p mnist --example mnist -- mnist '{"num_epochs": 5, "optimizer": {"weight_decay": 0.0001}}'
cargo run -p mnist --example mnist -- mnist -c config.json  # a JSON file merged onto the default
```

The example registers its `mnist` job, a function of `MnistTrainingConfig`, a Burn `Config`, with
`tracel::runtime::Jobs`. Its input, one JSON document or a `--config` file, is merged onto the
small default configuration. The configuration has no JSON Schema, so the job takes no flags; a job
whose input type derives `schemars::JsonSchema`, registered `with_schema()` with the `schema`
feature of `tracel`, takes a flag per field, as the `cli` example of [`basics`](../basics) shows.
The job trains in an experiment it runs with `Experiments::run`,
which records the merged configuration, the job's input, as the experiment's arguments and ends
the experiment as completed or failed by what training returns.

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
