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
```

Runs offline by default, so it needs no credentials. To ship metrics, checkpoints, and live progress
to the [console](https://console.tracel.ai), switch the connection in `examples/mnist.rs` to
`Connection::Cloud` and authenticate:

```bash
tracel login          # or set TRACEL_API_KEY
```

The namespace and name come from [`tracel.toml`](tracel.toml). Enable a backend with Cargo features
(defaults to `wgpu` and `flex`).
