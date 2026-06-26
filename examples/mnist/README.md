# Tracel SDK Example (MNIST)

This example shows how to adapt the standard Burn MNIST example into a Tracel SDK project. It trains a model and reports the experiment (metrics, checkpoints, and artifacts) to Tracel.

## What This Example Covers

- Defining a custom model artifact with `BundleEncode` and `BundleDecode`
- Exposing training configuration through `MnistTrainingConfig`
- Creating and running an experiment with `Context::new(Connection::Cloud)` and `ExperimentRun`
- Wiring metrics, checkpoints, and interruption handling through `ExperimentRun`
- Registering jobs and dispatching them through the `Cli`

## Project Layout

- [`examples/mnist-cli.rs`](examples/mnist-cli.rs): entry point that opens a cloud `Context`, registers experiment jobs, and dispatches them through the `Cli`
- [`src/training.rs`](src/training.rs): training loop, evaluation, and artifact upload
- [`src/model.rs`](src/model.rs): model definition and artifact bundle serialization
- [`src/data.rs`](src/data.rs): MNIST batching and data augmentation

## Run

`Context::new(Connection::Cloud)` needs Tracel credentials. Either set `TRACEL_API_KEY` in your environment, or authenticate once with:

```bash
burn login
```

The project's namespace and name come from [`tracel.toml`](tracel.toml). You need to enable a backend with Cargo features. You can set default features in your `Cargo.toml` (this example defaults to `wgpu` and `flex`).

Run the default job:

```bash
cargo run --example mnist-cli
```

Run a specific registered job by name, with an optional JSON config:

```bash
cargo run --example mnist-cli -- mnist_wgpu
cargo run --example mnist-cli -- mnist_wgpu '{"num_epochs": 5}'
```

You choose the experiment name when you create a job with `module.create("name", ...)`.

## Configuration Mappers

When you register a job with the `Cli`, you pair it with a **mapper** that converts the raw CLI input into your config type. The SDK provides three built-in mappers:

| Mapper         | Input       | Use case                                                                       |
| -------------- | ----------- | ------------------------------------------------------------------------------ |
| `JsonMapper`   | JSON string | Pass config as JSON. Supports a default config that gets merged with overrides. |
| `ClapMapper`   | CLI flags   | Parse config using `clap::Parser` (e.g. `--num-epochs 5 --batch-size 64`).     |
| `PresetMapper` | Preset name | Choose from a set of named configurations (e.g. `small`, `large`).             |

This example uses `JsonMapper::with_default(MnistTrainingConfig::default())`, so you can run with defaults or override specific fields:

```bash
cargo run --example mnist-cli -- mnist_wgpu '{"num_epochs": 5}'
```

You can also implement the `Mapper<I>` trait to define your own mapping logic.

## More Details

- Tracel SDK docs: [docs.rs/tracel](https://docs.rs/tracel/latest/tracel/)
