# Tracel SDK Example (MNIST)

Adapts the standard Burn MNIST example into a Tracel project. It trains a real model and reports
the experiment (metrics, checkpoints, progress, and artifacts) to Tracel — the one example that
shows the **Burn `train` integration** end to end.

For the framework's shape without Burn (streaming inference, a toy experiment, and the uniform
`Cli` / `Server` drivers), see the [`basics`](../basics) examples.

## What it covers

- A custom model artifact via `BundleEncode` / `BundleDecode` (`src/model.rs`).
- Training configuration through `MnistTrainingConfig`.
- Running an experiment with `Context::new(...).experiment()` and `ExperimentRun`.
- The Burn `train` adapters, wired in `src/training.rs` via `ExperimentTrainingExt`:
  - `metric_logger()` — training/validation metrics,
  - `checkpointers()` — model/optimizer/scheduler checkpoints,
  - `training_progress_logger()` — epoch/split **progress as experiment activities**,
  - `interrupter()` — cancellation.

## Run

```bash
cargo run -p mnist --example mnist
```

This runs **offline** by default (telemetry stubbed locally), so it needs no credentials. To ship
metrics, checkpoints, and live activity/progress to the dashboard, switch the connection in
`examples/mnist.rs` to `Connection::Cloud` and authenticate:

```bash
tracel login          # or set TRACEL_API_KEY
```

The namespace and name come from [`tracel.toml`](tracel.toml). Enable a backend with Cargo features
(defaults to `wgpu` + `flex`).

## More details

- Tracel SDK docs: [docs.rs/tracel](https://docs.rs/tracel/latest/tracel/)
