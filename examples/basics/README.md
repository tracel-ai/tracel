# basics

Small runnable examples of the Tracel framework, using toy capabilities:

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
| `cli` | A CLI serving both jobs, each with flags built from its input schema. |
| `serve` | An HTTP server serving both jobs (SSE for inference, fire-and-forget for training). |
| `infer-client` | Streaming HTTP client for `serve`. |
| `console` | The signed-in user and the project's models, read from the console. |

## Run

```sh
cargo run -p basics --example infer
cargo run -p basics --example train

cargo run -p basics --example cli -- --help                  # lists the jobs
cargo run -p basics --example cli -- --version               # prints cli 0.10.0
cargo run -p basics --example cli -- toy-training --help     # lists the job's flags
cargo run -p basics --example cli -- wordtok --text "hello streaming world"
cargo run -p basics --example cli -- toy-training --epochs 2 --batches-per-epoch 4
cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'  # the same input as JSON
cargo run -p basics --example cli -- --completions bash > cli.bash  # a bash completion script
TRACEL_DESCRIBE=jobs.json cargo run -p basics --example cli  # writes the job definitions to jobs.json
TRACEL_REPORT_FILE=report.json cargo run -p basics --example cli -- toy-training  # writes the job's run report to report.json

cargo run -p basics --example serve
curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
curl -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
cargo run -p basics --example infer-client
```
