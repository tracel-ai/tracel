# basics

Small runnable examples of the Tracel SDK, using toy capabilities:

- `WordTokenizer`, a streaming inference that splits a prompt into tokens.
- a stand-in training loop that tracks activities, logs metrics, and handles cancellation.

They run offline by default, recording under `./runs`, so no credentials are needed. Each example
reads its target with `tracel::Target::from_env`, which chooses it from `TRACEL_TARGET`:

```sh
cargo run -p basics --example train    # offline (default)
TRACEL_TARGET=console TRACEL_NAMESPACE=<owner> TRACEL_PROJECT=<project> \
    cargo run -p basics --example train  # ships to the console
```

`console` needs credentials: run `tracel login` or set `TRACEL_API_KEY` first.

The `mnist` example shows the same experiment tracking driven from the Burn `train` integration.

## Examples

| Example | Shows |
| --- | --- |
| `infer` | Streaming inference: prompts fed over time, tokens streamed back. |
| `train` | An experiment run: activity tracking, metrics, cancellation. |
| `cli` | A program with two jobs, plain functions with flags built from their input schema: the training loop in an experiment, and the tokenizer, which starts none. |
| `console` | The signed-in user and the project's models, read from the console. |

## Run

```sh
cargo run -p basics --example infer
cargo run -p basics --example train

cargo run -p basics --example cli -- --help                  # lists the jobs
cargo run -p basics --example cli -- --version               # prints cli 0.10.0
cargo run -p basics --example cli -- toy-training --help     # lists the job's flags
cargo run -p basics --example cli -- toy-training --epochs 2 --batches-per-epoch 4
cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'  # the same input as JSON
cargo run -p basics --example cli -- wordtok --text "hello streaming world"
cargo run -p basics --example cli -- --completions bash > cli.bash  # a bash completion script
```
