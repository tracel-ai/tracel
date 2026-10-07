# basics

Small runnable examples of the Tracel framework, using toy capabilities:

- `WordTokenizer`, a streaming inference that splits a prompt into tokens.
- a stand-in training loop that tracks activities, logs metrics, and handles cancellation.

They run offline by default, recording under `./runs`, so no credentials are needed. Each example
reads its target with `tracel::Target::from_env`, which chooses it from `TRACEL_CONNECTION`:

```sh
cargo run -p basics --example train    # offline (default)
TRACEL_CONNECTION=console TRACEL_NAMESPACE=<owner> TRACEL_PROJECT=<project> \
    cargo run -p basics --example train  # ships to the console
```

`console` needs credentials: run `tracel login` or set `TRACEL_API_KEY` first.

The `mnist` example shows the same experiment tracking driven from the Burn `train` integration.

## Examples

| Example | Shows |
| --- | --- |
| `infer` | Streaming inference: prompts fed over time, tokens streamed back. |
| `train` | An experiment run: activity tracking, metrics, cancellation. |
| `cli` | A CLI serving both jobs. |
| `serve` | An HTTP server serving both jobs (SSE for inference, fire-and-forget for training). |
| `infer-client` | Streaming HTTP client for `serve`. |
| `console` | The signed-in user and the project's models, read from the console. |

## Run

```sh
cargo run -p basics --example infer
cargo run -p basics --example train

cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'
TRACEL_DESCRIBE=jobs.json cargo run -p basics --example cli  # writes the job definitions to jobs.json

cargo run -p basics --example serve
curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
curl -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
cargo run -p basics --example infer-client
```
